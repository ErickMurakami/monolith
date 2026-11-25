//! Local navigation and cancellable, shallow directory reads for Monolith Files.

use std::{
    collections::VecDeque,
    ffi::OsString,
    fs, io,
    os::windows::fs::MetadataExt,
    path::{self, Component, Path, PathBuf, Prefix},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::HWND,
    Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN},
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

pub const READY: u32 = WM_APP + 11;
const ENTRY_LIMIT: usize = 50_000;
const HISTORY_LIMIT: usize = 128;

pub struct Entry {
    pub name: OsString,
    pub directory: bool,
    display: String,
}

impl Entry {
    pub fn display(&self) -> &str {
        &self.display
    }
}

#[derive(Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub incomplete: bool,
}

fn read(path: &Path, show_hidden: bool, cancelled: impl Fn() -> bool) -> io::Result<Listing> {
    let mut listing = Listing::default();
    for item in fs::read_dir(path)? {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let Ok((name, attributes)) = item.and_then(|item| {
            item.metadata()
                .map(|metadata| (item.file_name(), metadata.file_attributes()))
        }) else {
            listing.incomplete = true;
            continue;
        };
        if !show_hidden && attributes & FILE_ATTRIBUTE_HIDDEN != 0 {
            continue;
        }
        if listing.entries.len() == ENTRY_LIMIT {
            listing.incomplete = true;
            break;
        }
        listing.entries.push(Entry {
            display: name.to_string_lossy().into_owned(),
            name,
            directory: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        });
    }
    if cancelled() {
        return Err(io::Error::from(io::ErrorKind::Interrupted));
    }
    listing
        .entries
        .sort_by_cached_key(|entry| (!entry.directory, entry.display.to_lowercase()));
    Ok(listing)
}

pub fn resolve(base: &Path, input: &Path) -> io::Result<PathBuf> {
    let path = if input.is_absolute() {
        input.to_owned()
    } else {
        base.join(input)
    };
    if matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::DeviceNS(_) | Prefix::Verbatim(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Device paths cannot be browsed",
        ));
    }
    path::absolute(path)
}

#[derive(Clone, Copy)]
pub enum Move {
    Open,
    History(usize),
    Refresh,
}

#[derive(Default)]
pub struct Navigation {
    folders: VecDeque<PathBuf>,
    index: usize,
}

impl Navigation {
    pub fn current(&self) -> Option<&Path> {
        self.folders.get(self.index).map(PathBuf::as_path)
    }

    pub fn history(&self, forwards: bool) -> Option<(PathBuf, Move)> {
        let index = if forwards {
            self.index + 1
        } else {
            self.index.checked_sub(1)?
        };
        Some((self.folders.get(index)?.clone(), Move::History(index)))
    }

    pub fn commit(&mut self, path: PathBuf, movement: Move) {
        match movement {
            Move::History(index) => self.index = index,
            Move::Open if self.current() != Some(&path) => {
                self.folders.truncate(self.index + 1);
                self.folders.push_back(path);
                if self.folders.len() > HISTORY_LIMIT {
                    self.folders.pop_front();
                }
                self.index = self.folders.len() - 1;
            }
            _ => {}
        }
    }
}

struct Request {
    revision: u64,
    path: PathBuf,
    movement: Move,
    show_hidden: bool,
}

pub struct Completed {
    pub path: PathBuf,
    pub movement: Move,
    pub listing: io::Result<Listing>,
}

#[derive(Default)]
struct Pending {
    request: Option<Request>,
    completed: Option<Completed>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    revision: AtomicU64,
    pending: Mutex<Pending>,
    wake: Condvar,
}

pub struct Reader(Arc<Shared>);

impl Reader {
    pub fn start(window: HWND) -> io::Result<Self> {
        let window = window as usize;
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("monolith-files".into())
            .spawn(move || serve(window as HWND, &worker))?;
        Ok(Self(shared))
    }

    pub fn load(&self, path: PathBuf, movement: Move, show_hidden: bool) {
        let mut pending = self.0.pending.lock().unwrap();
        let revision = self.0.revision.fetch_add(1, Ordering::Relaxed) + 1;
        pending.request = Some(Request {
            revision,
            path,
            movement,
            show_hidden,
        });
        pending.completed = None;
        self.0.wake.notify_one();
    }

    pub fn completed(&self) -> Option<Completed> {
        self.0.pending.lock().unwrap().completed.take()
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let mut pending = self.0.pending.lock().unwrap();
        pending.closed = true;
        self.0.revision.fetch_add(1, Ordering::Relaxed);
        self.0.wake.notify_one();
    }
}

fn serve(window: HWND, shared: &Shared) {
    loop {
        let request = {
            let pending = shared.pending.lock().unwrap();
            let mut pending = shared
                .wake
                .wait_while(pending, |pending| {
                    !pending.closed && pending.request.is_none()
                })
                .unwrap();
            if pending.closed {
                return;
            }
            pending.request.take().unwrap()
        };
        let stale = || shared.revision.load(Ordering::Relaxed) != request.revision;
        let listing = read(&request.path, request.show_hidden, stale);
        let mut pending = shared.pending.lock().unwrap();
        if !pending.closed && !stale() {
            pending.completed = Some(Completed {
                path: request.path,
                movement: request.movement,
                listing,
            });
            unsafe { PostMessageW(window, READY, 0, 0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_folder_after_back_discards_the_forward_history() {
        let mut navigation = Navigation::default();
        navigation.commit(PathBuf::from(r"C:\one"), Move::Open);
        navigation.commit(PathBuf::from(r"C:\two"), Move::Open);
        let (path, action) = navigation.history(false).unwrap();
        navigation.commit(path, action);
        assert_eq!(navigation.current(), Some(Path::new(r"C:\one")));
        assert_eq!(navigation.history(true).unwrap().0, Path::new(r"C:\two"));
        navigation.commit(PathBuf::from(r"C:\three"), Move::Open);
        assert!(navigation.history(true).is_none());
    }

    #[test]
    fn accepts_unicode_unc_and_rejects_device_namespaces() {
        let base = Path::new(r"C:\home");
        assert_eq!(
            resolve(base, Path::new("música")).unwrap(),
            base.join("música")
        );
        assert!(resolve(base, Path::new(r"\\server\share\folder")).is_ok());
        assert!(resolve(base, Path::new(r"\\.\PhysicalDrive0")).is_err());
        assert!(resolve(base, Path::new(r"\\?\GLOBALROOT\Device")).is_err());
    }
}
