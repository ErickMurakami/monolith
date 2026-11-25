//! Event-driven bridge to the current Windows media session.

use crate::artwork::Artwork;
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use windows::{
    Foundation::TypedEventHandler,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    },
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::AgileReference,
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const READY: u32 = WM_APP + 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowPlaying {
    pub generation: u64,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub toggle: bool,
    pub previous: bool,
    pub next: bool,
    pub timeline: Option<Timeline>,
    pub artwork: Option<Arc<Artwork>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    start: i64,
    duration: i64,
    position: i64,
    measured_at: i64,
    rate_milli: Option<i32>,
}

impl Timeline {
    pub fn position_at(self, now: i64, playing: bool) -> (u32, bool) {
        let rate = self
            .rate_milli
            .filter(|_| playing && self.measured_at > 0)
            .unwrap_or(0);
        let elapsed = now.saturating_sub(self.measured_at).max(0);
        let advance = i128::from(elapsed) * i128::from(rate) / 1000;
        let duration = i128::from(self.duration);
        let position =
            (i128::from(self.position) - i128::from(self.start) + advance).clamp(0, duration);
        let continues = (rate > 0 && position < duration) || (rate < 0 && position > 0);
        ((position * 1000 / duration) as u32, continues)
    }
}

#[derive(Clone, Copy)]
pub enum Action {
    Toggle,
    Previous,
    Next,
}

enum Command {
    Refresh,
    Control {
        generation: u64,
        action: Action,
    },
    Artwork {
        key: ArtworkKey,
        image: Option<Arc<Artwork>>,
    },
}

type ArtworkKey = (u64, u64);

#[derive(Clone)]
struct Signals {
    sender: SyncSender<Command>,
    refresh_pending: Arc<AtomicBool>,
    artwork_demand: Arc<AtomicBool>,
    metadata_changed: Arc<AtomicBool>,
}

impl Signals {
    fn refresh(&self) {
        if !self.refresh_pending.swap(true, Ordering::AcqRel)
            && self.sender.try_send(Command::Refresh).is_err()
        {
            self.refresh_pending.store(false, Ordering::Release);
        }
    }
}

pub struct Media {
    signals: Signals,
    alive: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Option<NowPlaying>>>,
}

impl Media {
    pub fn connect(window: HWND) -> io::Result<Self> {
        let (sender, inbox) = mpsc::sync_channel(12);
        let signals = Signals {
            sender,
            refresh_pending: Arc::new(AtomicBool::new(false)),
            artwork_demand: Arc::new(AtomicBool::new(false)),
            metadata_changed: Arc::new(AtomicBool::new(false)),
        };
        let alive = Arc::new(AtomicBool::new(true));
        let snapshot = Arc::new(Mutex::new(None));
        let worker_signals = signals.clone();
        let worker_alive = Arc::clone(&alive);
        let publication = Arc::clone(&snapshot);
        let destination = window as isize;
        thread::Builder::new()
            .name("monolith-media".into())
            .spawn(move || {
                if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok() {
                    run(
                        inbox,
                        worker_signals,
                        &worker_alive,
                        &publication,
                        destination,
                    );
                    unsafe { RoUninitialize() };
                }
            })?;
        Ok(Self {
            signals,
            alive,
            snapshot,
        })
    }

    pub fn read(&self) -> Option<NowPlaying> {
        self.snapshot.lock().ok().and_then(|state| state.clone())
    }

    pub fn perform(&self, generation: u64, action: Action) {
        let _ = self
            .signals
            .sender
            .try_send(Command::Control { generation, action });
    }

    pub fn request_artwork(&self, enabled: bool) {
        if self.signals.artwork_demand.swap(enabled, Ordering::AcqRel) != enabled {
            self.signals.refresh();
        }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.signals.refresh();
    }
}

struct Subscription {
    session: Session,
    properties: Option<i64>,
    playback: Option<i64>,
    timeline: Option<i64>,
}

impl Subscription {
    fn attach(session: Session, signals: &Signals) -> Self {
        let metadata = signals.clone();
        let properties = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                metadata.metadata_changed.store(true, Ordering::Release);
                metadata.refresh();
                Ok(())
            }))
            .ok();
        let controls = signals.clone();
        let playback = session
            .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                controls.refresh();
                Ok(())
            }))
            .ok();
        let position = signals.clone();
        let timeline = session
            .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                position.refresh();
                Ok(())
            }))
            .ok();
        Self {
            session,
            properties,
            playback,
            timeline,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(token) = self.properties {
            let _ = self.session.RemoveMediaPropertiesChanged(token);
        }
        if let Some(token) = self.playback {
            let _ = self.session.RemovePlaybackInfoChanged(token);
        }
        if let Some(token) = self.timeline {
            let _ = self.session.RemoveTimelinePropertiesChanged(token);
        }
    }
}

fn run(
    inbox: Receiver<Command>,
    signals: Signals,
    alive: &AtomicBool,
    output: &Mutex<Option<NowPlaying>>,
    window: isize,
) {
    let Ok(manager) = Manager::RequestAsync().and_then(|operation| operation.join()) else {
        return;
    };
    let changes = signals.clone();
    let current_changed = manager
        .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            changes.refresh();
            Ok(())
        }))
        .ok();
    let mut active: Option<Subscription> = None;
    let mut generation = 0_u64;
    let mut metadata_revision = 0_u64;
    let mut artwork_cache: Option<(ArtworkKey, Option<Arc<Artwork>>)> = None;
    let mut decoding = false;
    signals.refresh();
    while alive.load(Ordering::Acquire) {
        let Ok(command) = inbox.recv() else { break };
        if !alive.load(Ordering::Acquire) {
            break;
        }
        if matches!(command, Command::Refresh) {
            signals.refresh_pending.store(false, Ordering::Release);
        }
        let current = manager.GetCurrentSession().ok();
        if active.as_ref().map(|subscription| &subscription.session) != current.as_ref() {
            active = current.map(|session| Subscription::attach(session, &signals));
            generation = generation.wrapping_add(1);
        }
        if signals.metadata_changed.swap(false, Ordering::AcqRel) {
            metadata_revision = metadata_revision.wrapping_add(1);
        }
        let key = (generation, metadata_revision);
        let wants_artwork = signals.artwork_demand.load(Ordering::Acquire);
        match command {
            Command::Control {
                generation: selected,
                action,
            } if generation == selected => {
                if let Some(active) = &active {
                    perform(&active.session, action);
                }
            }
            Command::Artwork {
                key: finished,
                image,
            } => {
                decoding = false;
                if wants_artwork && finished == key {
                    artwork_cache = Some((finished, image));
                }
            }
            _ => {}
        }
        if !wants_artwork {
            artwork_cache = None;
        }
        let snapshot = active
            .as_ref()
            .and_then(|active| read(&active.session, generation))
            .map(|(mut now_playing, properties)| {
                if wants_artwork {
                    if let Some((_, artwork)) =
                        artwork_cache.as_ref().filter(|(cached, _)| *cached == key)
                    {
                        now_playing.artwork = artwork.clone();
                    } else if !decoding {
                        decoding = start_artwork(&properties, key, signals.sender.clone());
                        if !decoding {
                            artwork_cache = Some((key, None));
                        }
                    }
                }
                now_playing
            });
        if let Ok(mut published) = output.lock()
            && *published != snapshot
        {
            *published = snapshot;
            drop(published);
            if alive.load(Ordering::Acquire) {
                unsafe { PostMessageW(window as HWND, READY, 0, 0) };
            }
        }
    }
    drop(active);
    if let Some(token) = current_changed {
        let _ = manager.RemoveCurrentSessionChanged(token);
    }
}

fn start_artwork(
    properties: &MediaProperties,
    key: ArtworkKey,
    sender: SyncSender<Command>,
) -> bool {
    let Ok(stream) = properties.Thumbnail() else {
        return false;
    };
    let Ok(stream) = AgileReference::new(&stream) else {
        return false;
    };
    thread::Builder::new()
        .name("monolith-artwork".into())
        .spawn(move || {
            let mut image = None;
            if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok() {
                image = stream
                    .resolve()
                    .ok()
                    .and_then(|reference| Artwork::decode(&reference))
                    .map(Arc::new);
                unsafe { RoUninitialize() };
            }
            let _ = sender.send(Command::Artwork { key, image });
        })
        .is_ok()
}

fn read(session: &Session, generation: u64) -> Option<(NowPlaying, MediaProperties)> {
    let media = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = media.Title().ok()?.to_string_lossy();
    let artist = media
        .Artist()
        .map_or_else(|_| String::new(), |text| text.to_string_lossy());
    if title.trim().is_empty() && artist.trim().is_empty() {
        return None;
    }
    let playback = session.GetPlaybackInfo().ok()?;
    let flags = playback.Controls().ok()?;
    let playing = playback.PlaybackStatus().ok() == Some(Status::Playing);
    let rate = playback
        .PlaybackRate()
        .and_then(|rate| rate.Value())
        .ok()
        .filter(|rate| rate.is_finite() && rate.abs() <= 1000.0)
        .map(|rate| (rate * 1000.0).round() as i32);
    let snapshot = NowPlaying {
        generation,
        title: title.chars().take(180).collect(),
        artist: artist.chars().take(100).collect(),
        playing,
        toggle: flags.IsPlayPauseToggleEnabled().unwrap_or(false)
            || if playing {
                flags.IsPauseEnabled().unwrap_or(false)
            } else {
                flags.IsPlayEnabled().unwrap_or(false)
            },
        previous: flags.IsPreviousEnabled().unwrap_or(false),
        next: flags.IsNextEnabled().unwrap_or(false),
        timeline: read_timeline(session, rate),
        artwork: None,
    };
    Some((snapshot, media))
}

fn read_timeline(session: &Session, rate_milli: Option<i32>) -> Option<Timeline> {
    let value = session.GetTimelineProperties().ok()?;
    let start = value.StartTime().ok()?.Duration;
    let end = value.EndTime().ok()?.Duration;
    let duration = end.checked_sub(start).filter(|duration| *duration > 0)?;
    let position = value.Position().ok()?.Duration.clamp(start, end);
    let measured_at = value.LastUpdatedTime().map_or(0, |time| time.UniversalTime);
    Some(Timeline {
        start,
        duration,
        position,
        measured_at,
        rate_milli,
    })
}

fn perform(session: &Session, action: Action) {
    let Ok(info) = session.GetPlaybackInfo() else {
        return;
    };
    let Ok(controls) = info.Controls() else {
        return;
    };
    let operation = match action {
        Action::Previous if controls.IsPreviousEnabled().unwrap_or(false) => {
            session.TrySkipPreviousAsync()
        }
        Action::Next if controls.IsNextEnabled().unwrap_or(false) => session.TrySkipNextAsync(),
        Action::Toggle if controls.IsPlayPauseToggleEnabled().unwrap_or(false) => {
            session.TryTogglePlayPauseAsync()
        }
        Action::Toggle => {
            if info.PlaybackStatus().ok() == Some(Status::Playing) {
                if !controls.IsPauseEnabled().unwrap_or(false) {
                    return;
                }
                session.TryPauseAsync()
            } else {
                if !controls.IsPlayEnabled().unwrap_or(false) {
                    return;
                }
                session.TryPlayAsync()
            }
        }
        _ => return,
    };
    if let Ok(operation) = operation {
        let _ = operation.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_using_reported_rate_and_freezes_while_paused() {
        let timeline = Timeline {
            start: 10_000_000,
            duration: 600_000_000,
            position: 110_000_000,
            measured_at: 2_000_000_000,
            rate_milli: Some(1500),
        };
        assert_eq!(timeline.position_at(2_040_000_000, true), (266, true));
        assert_eq!(timeline.position_at(2_040_000_000, false), (166, false));
        assert_eq!(timeline.position_at(i64::MAX, true), (1000, false));
        assert_eq!(
            Timeline {
                rate_milli: None,
                ..timeline
            }
            .position_at(2_040_000_000, true),
            (166, false)
        );
    }
}
