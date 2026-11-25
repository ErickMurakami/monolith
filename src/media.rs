//! Event-driven bridge to the current Windows media session.

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
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    },
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
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
}

#[derive(Clone, Copy)]
pub enum Action {
    Toggle,
    Previous,
    Next,
}

enum Command {
    Refresh,
    Control { generation: u64, action: Action },
}

#[derive(Clone)]
struct Signals {
    sender: SyncSender<Command>,
    refresh_pending: Arc<AtomicBool>,
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
}

impl Subscription {
    fn attach(session: Session, signals: &Signals) -> Self {
        let metadata = signals.clone();
        let properties = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
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
        Self {
            session,
            properties,
            playback,
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
        if let Command::Control {
            generation: selected,
            action,
        } = command
            && generation == selected
            && let Some(active) = &active
        {
            perform(&active.session, action);
        }
        let snapshot = active
            .as_ref()
            .and_then(|active| read(&active.session, generation));
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

fn read(session: &Session, generation: u64) -> Option<NowPlaying> {
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
    Some(NowPlaying {
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
