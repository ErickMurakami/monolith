//! Native file transfers for Files. Perform them off the UI thread.

use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, PathBuf, Prefix},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::E_ABORT,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            SystemServices::{SFGAO_FILESYSTEM, SFGAO_FOLDER},
        },
        UI::Shell::{
            COPYENGINE_S_NOT_HANDLED, COPYENGINE_S_USER_IGNORED, FILEOPERATION_FLAGS,
            FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_RENAMEONCOLLISION, FOFX_ADDUNDORECORD,
            FOFX_EARLYFAILURE, FOFX_NOCOPYHOOKS, FOFX_PRESERVEFILEEXTENSIONS, FileOperation,
            IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl,
            IShellItem, SHCreateItemFromParsingName, SICHINT_CANONICAL,
        },
    },
    core::{HRESULT, PCWSTR, Ref, implement},
};
use windows_sys::Win32::Foundation::HWND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOutcome {
    Complete,
    /// Cancelled, skipped, or failed after some of the batch could have executed.
    Interrupted,
}

pub fn perform(
    window: HWND,
    paths: &[PathBuf],
    destination: &Path,
    move_items: bool,
    cancel: Arc<AtomicBool>,
) -> io::Result<TransferOutcome> {
    execute(
        window,
        paths,
        destination,
        move_items,
        FOFX_ADDUNDORECORD,
        cancel,
    )
}

fn execute(
    window: HWND,
    paths: &[PathBuf],
    destination: &Path,
    move_items: bool,
    extra: FILEOPERATION_FLAGS,
    cancel: Arc<AtomicBool>,
) -> io::Result<TransferOutcome> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No files were selected",
        ));
    }
    // Initialise COM on the worker that owns the Shell operation and its callbacks.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }.map_err(native_error)?;
    let _apartment = Apartment;
    let folder = shell_item(destination)?;
    if !unsafe { folder.GetAttributes(SFGAO_FOLDER) }
        .map_err(native_error)?
        .contains(SFGAO_FOLDER)
    {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "The destination is not a folder",
        ));
    }
    let operation: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER) }
            .map_err(native_error)?;
    let flags = FOF_RENAMEONCOLLISION
        | FOFX_PRESERVEFILEEXTENSIONS
        | FOF_NOCONFIRMMKDIR
        | FOF_NOERRORUI
        | FOFX_EARLYFAILURE
        | FOFX_NOCOPYHOOKS
        | extra;
    unsafe { operation.SetOperationFlags(flags) }.map_err(native_error)?;
    if !window.is_null() {
        unsafe { operation.SetOwnerWindow(windows::Win32::Foundation::HWND(window)) }
            .map_err(native_error)?;
    }
    // Queueing never writes. Validate every item before the first operation can run.
    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            return Ok(TransferOutcome::Interrupted);
        }
        let item = shell_item(path)?;
        if move_items {
            let parent = unsafe { item.GetParent() }.map_err(native_error)?;
            if unsafe { parent.Compare(&folder, SICHINT_CANONICAL.0 as u32) }
                .map_err(native_error)?
                == 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "A selected item is already in the destination folder",
                ));
            }
            unsafe { operation.MoveItem(&item, &folder, PCWSTR::null(), None) }
                .map_err(native_error)?;
        } else {
            unsafe { operation.CopyItem(&item, &folder, PCWSTR::null(), None) }
                .map_err(native_error)?;
        }
    }
    let incomplete = Arc::new(AtomicBool::new(false));
    let observer: IFileOperationProgressSink = Observer {
        incomplete: Arc::clone(&incomplete),
        cancel,
    }
    .into();
    let cookie = unsafe { operation.Advise(&observer) }.map_err(native_error)?;
    let result = unsafe { operation.PerformOperations() };
    // The Shell can return S_OK after cancellation. Query aborts even if it failed.
    let aborted = unsafe { operation.GetAnyOperationsAborted() };
    let _ = unsafe { operation.Unadvise(cookie) };
    if incomplete.load(Ordering::Relaxed) || aborted.as_ref().is_ok_and(|aborted| aborted.as_bool())
    {
        return Ok(TransferOutcome::Interrupted);
    }
    result.map_err(native_error)?;
    let _ = aborted.map_err(native_error)?;
    Ok(TransferOutcome::Complete)
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn shell_item(path: &Path) -> io::Result<IShellItem> {
    let local_path = matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(),
        Prefix::Disk(_) | Prefix::UNC(_, _) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)));
    let streams = path.components().any(|part| matches!(part, Component::Normal(name) if name.encode_wide().any(|c| c == b':' as u16)));
    let mut path_wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if !local_path || !path.is_absolute() || streams || path_wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "An absolute Windows file path is required",
        ));
    }
    path_wide.push(0);
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None) }
        .map_err(native_error)?;
    if !unsafe { item.GetAttributes(SFGAO_FILESYSTEM) }
        .map_err(native_error)?
        .contains(SFGAO_FILESYSTEM)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The item is not a file system object",
        ));
    }
    Ok(item)
}

fn native_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code & 0xffff_0000 == 0x8007_0000 {
        return io::Error::from_raw_os_error((code & 0xffff) as i32);
    }
    io::Error::other(format!("Windows file transfer: {error}"))
}

#[implement(IFileOperationProgressSink)]
struct Observer {
    incomplete: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl Observer {
    fn check_cancelled(&self) -> windows::core::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            self.incomplete.store(true, Ordering::Relaxed);
            return Err(windows::core::Error::from_hresult(E_ABORT));
        }
        Ok(())
    }

    fn record(&self, result: HRESULT) {
        if result.is_err() || matches!(result, COPYENGINE_S_NOT_HANDLED | COPYENGINE_S_USER_IGNORED)
        {
            self.incomplete.store(true, Ordering::Relaxed);
        }
    }
}

impl IFileOperationProgressSink_Impl for Observer_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        self.check_cancelled()
    }
    fn FinishOperations(&self, result: HRESULT) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.check_cancelled()
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.check_cancelled()
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreDeleteItem(&self, _: u32, _: Ref<IShellItem>) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        result: HRESULT,
        _: Ref<IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(result);
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
        self.check_cancelled()
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs, os::windows::fs::OpenOptionsExt, ptr::null_mut, time::SystemTime};
    use windows::Win32::UI::Shell::FOF_SILENT;

    struct Fixture(PathBuf);

    impl Fixture {
        fn create() -> Self {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            // Parallel tests can share a timestamp; the counter keeps fixtures distinct.
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = env::temp_dir().join(format!(
                "monolith-file-transfer-{}-{stamp}-{serial}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("from")).unwrap();
            fs::create_dir(root.join("to")).unwrap();
            Self(root)
        }

        fn source(&self, file: &str) -> PathBuf {
            self.0.join("from").join(file)
        }
        fn destination(&self, file: &str) -> PathBuf {
            self.0.join("to").join(file)
        }

        fn transfer(&self, paths: &[PathBuf], move_items: bool) -> io::Result<TransferOutcome> {
            execute(
                null_mut(),
                paths,
                &self.destination(""),
                move_items,
                FOF_SILENT,
                Arc::new(AtomicBool::new(false)),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let (Ok(directory), Ok(scratch)) =
                (fs::canonicalize(&self.0), fs::canonicalize(env::temp_dir()))
                && directory.parent() == Some(scratch.as_path())
                && directory.file_name() == self.0.file_name()
            {
                let _ = fs::remove_dir_all(directory);
            }
        }
    }

    #[test]
    fn keeps_existing_files_when_copying_and_moving() {
        let fixture = Fixture::create();
        let source = fixture.source("áudio.bin");
        let existing = fixture.destination("áudio.bin");
        fs::write(&source, b"copy").unwrap();
        fs::write(&existing, b"existing").unwrap();
        assert_eq!(
            fixture
                .transfer(std::slice::from_ref(&source), false)
                .unwrap(),
            TransferOutcome::Complete
        );
        assert_eq!(fs::read(&source).unwrap(), b"copy");
        fs::write(&source, b"move").unwrap();
        assert_eq!(
            fixture
                .transfer(std::slice::from_ref(&source), true)
                .unwrap(),
            TransferOutcome::Complete
        );
        assert!(!source.exists());
        assert_eq!(fs::read(&existing).unwrap(), b"existing");
        let mut bytes: Vec<_> = fs::read_dir(fixture.destination(""))
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                assert_eq!(path.extension().unwrap(), "bin");
                fs::read(path).unwrap()
            })
            .collect();
        bytes.sort();
        assert_eq!(
            bytes,
            [b"copy".to_vec(), b"existing".to_vec(), b"move".to_vec()]
        );
    }

    #[test]
    fn preserves_destination_folder_contents_at_nested_collisions() {
        let fixture = Fixture::create();
        let source = fixture.source("folder");
        let existing = fixture.destination("folder");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&existing).unwrap();
        fs::write(source.join("collision.txt"), b"source").unwrap();
        fs::write(existing.join("collision.txt"), b"existing").unwrap();
        assert_eq!(
            fixture.transfer(&[source], false).unwrap(),
            TransferOutcome::Complete
        );
        assert_eq!(
            fs::read(existing.join("collision.txt")).unwrap(),
            b"existing"
        );
        let found = fs::read_dir(fixture.destination("")).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path != existing
                && fs::read(path.join("collision.txt")).is_ok_and(|bytes| bytes == b"source")
        });
        assert!(
            found,
            "Windows did not keep the two colliding folders separately"
        );
    }

    #[test]
    fn reports_native_failure_for_a_locked_item_in_a_batch() {
        let fixture = Fixture::create();
        let ordinary = fixture.source("a.txt");
        let restricted = fixture.source("z.txt");
        fs::write(&ordinary, b"movable").unwrap();
        fs::write(&restricted, b"locked").unwrap();
        let _locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&restricted)
            .unwrap();
        assert_ne!(
            fixture.transfer(&[ordinary, restricted], true).ok(),
            Some(TransferOutcome::Complete)
        );
        assert!(fixture.source("z.txt").exists());
        assert!(!fixture.destination("z.txt").exists());
    }

    #[test]
    fn an_already_cancelled_move_preserves_the_source() {
        let fixture = Fixture::create();
        let source = fixture.source("cancel.txt");
        fs::write(&source, b"keep").unwrap();
        assert_eq!(
            execute(
                null_mut(),
                std::slice::from_ref(&source),
                &fixture.destination(""),
                true,
                FOF_SILENT,
                Arc::new(AtomicBool::new(true)),
            )
            .unwrap(),
            TransferOutcome::Interrupted
        );
        assert_eq!(fs::read(source).unwrap(), b"keep");
        assert!(!fixture.destination("cancel.txt").exists());
    }

    /// A uniquely named folder on another volume, removed only if the name still matches.
    struct Remote(PathBuf);

    impl Drop for Remote {
        fn drop(&mut self) {
            if self
                .0
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("monolith-transfer-"))
            {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    #[ignore = "needs a second fixed volume D: or the running archlinux WSL distribution"]
    fn transfers_across_volumes_and_into_wsl() {
        let mut exercised = 0;
        for (index, root) in [r"D:\", r"\\wsl.localhost\archlinux\tmp"]
            .iter()
            .enumerate()
        {
            let root = Path::new(root);
            if !root.is_dir() {
                eprintln!("skipped unavailable {}", root.display());
                continue;
            }
            let fixture = Fixture::create();
            let remote =
                Remote(root.join(format!("monolith-transfer-{}-{index}", std::process::id())));
            fs::create_dir(&remote.0).unwrap();
            let run = |paths: &[PathBuf], destination: &Path, move_items| {
                execute(
                    null_mut(),
                    paths,
                    destination,
                    move_items,
                    FOF_SILENT,
                    Arc::new(AtomicBool::new(false)),
                )
                .unwrap()
            };
            let copied = fixture.source("café.txt");
            fs::write(&copied, b"copied").unwrap();
            assert_eq!(
                run(std::slice::from_ref(&copied), &remote.0, false),
                TransferOutcome::Complete
            );
            assert_eq!(fs::read(remote.0.join("café.txt")).unwrap(), b"copied");
            assert!(copied.exists());

            // Across volumes the Shell copies and then deletes the source.
            let moved = fixture.source("moved.txt");
            fs::write(&moved, b"moved").unwrap();
            assert_eq!(
                run(std::slice::from_ref(&moved), &remote.0, true),
                TransferOutcome::Complete
            );
            assert!(!moved.exists());
            assert_eq!(fs::read(remote.0.join("moved.txt")).unwrap(), b"moved");

            let back = remote.0.join("café.txt");
            assert_eq!(
                run(&[back], &fixture.destination(""), false),
                TransferOutcome::Complete
            );
            assert_eq!(
                fs::read(fixture.destination("café.txt")).unwrap(),
                b"copied"
            );
            eprintln!("transferred across {}", root.display());
            exercised += 1;
        }
        assert!(
            exercised > 0,
            "no secondary volume or WSL distribution was available"
        );
    }

    #[test]
    #[ignore = "needs a second fixed volume D:"]
    fn cancelling_a_cross_volume_move_midway_loses_no_file() {
        const COUNT: usize = 300;
        let root = Path::new(r"D:\");
        if !root.is_dir() {
            eprintln!("skipped: D: is unavailable");
            return;
        }
        let fixture = Fixture::create();
        let remote = Remote(root.join(format!("monolith-transfer-{}-cancel", std::process::id())));
        fs::create_dir(&remote.0).unwrap();
        let sources: Vec<PathBuf> = (0..COUNT)
            .map(|index| {
                let path = fixture.source(&format!("item-{index:03}.bin"));
                fs::write(&path, vec![index as u8; 64 * 1024]).unwrap();
                path
            })
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = {
            let (sources, destination, cancel) =
                (sources.clone(), remote.0.clone(), Arc::clone(&cancel));
            std::thread::spawn(move || {
                execute(null_mut(), &sources, &destination, true, FOF_SILENT, cancel)
            })
        };
        // Cancel once the move is under way.
        while fs::read_dir(&remote.0).unwrap().count() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            worker.join().unwrap().unwrap(),
            TransferOutcome::Interrupted
        );
        let mut moved = 0;
        for (index, source) in sources.iter().enumerate() {
            let destination = remote.0.join(source.file_name().unwrap());
            let (at_source, at_destination) = (source.exists(), destination.exists());
            assert!(
                at_source ^ at_destination,
                "item {index} exists {at_source}/{at_destination}"
            );
            let kept = if at_source { source } else { &destination };
            assert_eq!(fs::read(kept).unwrap(), vec![index as u8; 64 * 1024]);
            moved += usize::from(at_destination);
        }
        assert!(moved < COUNT, "the move finished before cancellation");
        eprintln!("cancelled after moving {moved} of {COUNT} items");
    }
}
