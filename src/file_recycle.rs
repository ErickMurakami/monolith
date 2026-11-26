//! Native, recycle-only operations for Files. Call `perform` on the Files worker.
//!
//! The Shell chooses whether it can accept an item. Monolith asks the filesystem
//! transfer source to recycle it and propagates rejection without retrying through
//! a permanent deletion API. The initial scope is ordinary items on fixed local
//! volumes. Cancellation is accepted between items.

use std::{
    fs, io,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf, Prefix},
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            SystemServices::SFGAO_FILESYSTEM,
        },
        UI::Shell::{
            BHID_Transfer, COPYENGINE_E_RECYCLE_BIN_NOT_FOUND, COPYENGINE_E_RECYCLE_FORCE_NUKE,
            COPYENGINE_E_RECYCLE_PATH_TOO_LONG, COPYENGINE_E_RECYCLE_SIZE_TOO_BIG,
            COPYENGINE_E_RECYCLE_UNKNOWN_ERROR, COPYENGINE_E_USER_CANCELLED,
            FOLDERID_RecycleBinFolder, IShellItem, ITransferAdviseSink, ITransferAdviseSink_Impl,
            ITransferSource, KNOWN_FOLDER_FLAG, SHCreateItemFromParsingName, SHGetKnownFolderItem,
            TSF_NORMAL,
        },
    },
    core::{Error, HRESULT, PCWSTR, PWSTR, Ref, implement},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, GetDriveTypeW, GetVolumePathNameW,
};

const DRIVE_FIXED: u32 = 3;

pub struct RecycleOutcome {
    pub recycled: usize,
    /// The remaining selection was interrupted. Earlier successful items are in the bin.
    pub error: Option<io::Error>,
}

/// Setup and preflight errors leave the whole selection untouched. Once recycling
/// begins, report the number confirmed in the bin alongside any interruption.
pub fn perform(paths: &[PathBuf], cancel: &AtomicBool) -> io::Result<RecycleOutcome> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No items were selected",
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(interrupted(0, cancelled()));
    }
    let _apartment = Apartment::new()?;
    let bin = recycle_bin()?;
    let mut volume = vec![0; 32_768];
    let mut items = Vec::with_capacity(paths.len());
    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            return Ok(interrupted(0, cancelled()));
        }
        items.push(recycle_source(path, &mut volume)?);
    }
    for (index, (item, source)) in items.into_iter().enumerate() {
        if let Err(error) = recycle_item(&source, &item, &bin, cancel) {
            return Ok(interrupted(index, error));
        }
    }
    Ok(RecycleOutcome {
        recycled: paths.len(),
        error: None,
    })
}

fn interrupted(recycled: usize, error: io::Error) -> RecycleOutcome {
    RecycleOutcome {
        recycled,
        error: Some(error),
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Recycling was cancelled")
}

struct Apartment;

impl Apartment {
    fn new() -> io::Result<Self> {
        // Both S_OK and S_FALSE require a matching CoUninitialize call.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }.map_err(native_error)?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn recycle_bin() -> io::Result<IShellItem> {
    // Obtain the virtual folder from the Shell. Its on-disk format is implementation-owned.
    unsafe { SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KNOWN_FOLDER_FLAG(0), None) }
        .map_err(native_error)
}

fn recycle_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut components = path.components();
    let disk = matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(
        prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)
    ));
    if !disk || !path.is_absolute() || path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Recycling requires an item on a fixed local volume",
        ));
    }
    for component in components {
        match component {
            Component::Normal(name)
                if !name.eq_ignore_ascii_case("$Recycle.Bin")
                    && !name.encode_wide().any(|unit| unit == u16::from(b':')) => {}
            Component::RootDir => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "This item cannot be recycled",
                ));
            }
        }
    }
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid file name",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn recycle_source(path: &Path, volume: &mut [u16]) -> io::Result<(IShellItem, ITransferSource)> {
    let wide = recycle_path(path)?;
    if unsafe { GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { GetDriveTypeW(volume.as_ptr()) } != DRIVE_FIXED {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Recycling requires an item on a fixed local volume",
        ));
    }
    if fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Recycling reparse points is unavailable",
        ));
    }
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
        .map_err(native_error)?;
    if !unsafe { item.GetAttributes(SFGAO_FILESYSTEM) }
        .map_err(native_error)?
        .contains(SFGAO_FILESYSTEM)
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Only filesystem items can be recycled",
        ));
    }
    let parent = unsafe { item.GetParent() }.map_err(native_error)?;
    let source = unsafe { parent.BindToHandler(None, &BHID_Transfer) }.map_err(native_error)?;
    Ok((item, source))
}

fn recycle_item(
    source: &ITransferSource,
    item: &IShellItem,
    bin: &IShellItem,
    cancel: &AtomicBool,
) -> io::Result<IShellItem> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    let observer: ITransferAdviseSink = RecycleObserver.into();
    let cookie = unsafe { source.Advise(&observer) }.map_err(native_error)?;
    // RecycleItem is distinct from RemoveItem. TSF_NORMAL permits neither
    // overwriting nor decrypting, and adds no conditional-delete operation.
    let result = unsafe { source.RecycleItem(item, bin, TSF_NORMAL.0 as u32) };
    let _ = unsafe { source.Unadvise(cookie) };
    // windows-rs rejects a null result even if the Shell returns a success HRESULT.
    result.map_err(native_error)
}

fn native_error(error: Error) -> io::Error {
    match error.code() {
        COPYENGINE_E_USER_CANCELLED => cancelled(),
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND | COPYENGINE_E_RECYCLE_FORCE_NUKE => io::Error::new(
            io::ErrorKind::Unsupported,
            "The Recycle Bin is unavailable for this item",
        ),
        COPYENGINE_E_RECYCLE_PATH_TOO_LONG => io::Error::new(
            io::ErrorKind::InvalidInput,
            "The item path is too long for the Recycle Bin",
        ),
        COPYENGINE_E_RECYCLE_SIZE_TOO_BIG => io::Error::new(
            io::ErrorKind::InvalidInput,
            "The item is too large for the Recycle Bin",
        ),
        COPYENGINE_E_RECYCLE_UNKNOWN_ERROR => {
            io::Error::other("Windows refused to recycle the item")
        }
        code if (code.0 as u32) & 0xffff_0000 == 0x8007_0000 => {
            io::Error::from_raw_os_error(code.0 & 0xffff)
        }
        _ => io::Error::other(format!("Windows Recycle Bin: {error}")),
    }
}

#[implement(ITransferAdviseSink)]
struct RecycleObserver;

fn refuse(error: HRESULT) -> windows::core::Result<()> {
    Err(Error::from_hresult(if error.is_err() {
        error
    } else {
        COPYENGINE_E_USER_CANCELLED
    }))
}

impl ITransferAdviseSink_Impl for RecycleObserver_Impl {
    fn UpdateProgress(
        &self,
        _: u64,
        _: u64,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn UpdateTransferState(&self, _: u32) -> windows::core::Result<()> {
        Ok(())
    }

    fn ConfirmOverwrite(
        &self,
        _: Ref<IShellItem>,
        _: Ref<IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        refuse(COPYENGINE_E_USER_CANCELLED)
    }

    fn ConfirmEncryptionLoss(&self, _: Ref<IShellItem>) -> windows::core::Result<()> {
        refuse(COPYENGINE_E_USER_CANCELLED)
    }

    fn FileFailure(
        &self,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        error: HRESULT,
        _: PWSTR,
        _: u32,
    ) -> windows::core::Result<()> {
        refuse(error)
    }

    fn SubStreamFailure(
        &self,
        _: Ref<IShellItem>,
        _: &PCWSTR,
        error: HRESULT,
    ) -> windows::core::Result<()> {
        refuse(error)
    }

    fn PropertyFailure(
        &self,
        _: Ref<IShellItem>,
        _: *const PROPERTYKEY,
        error: HRESULT,
    ) -> windows::core::Result<()> {
        refuse(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs::OpenOptions, io::Write, time::SystemTime};
    use windows::Win32::{
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::Shell::{
            FOF_NOERRORUI, FOF_RENAMEONCOLLISION, FOF_SILENT, FOFX_EARLYFAILURE, FileOperation,
            IFileOperation,
        },
    };

    #[test]
    fn rejects_network_wsl_device_roots_streams_and_existing_bin_items() {
        for path in [
            r"\\files.example\share\document.txt",
            r"\\wsl.localhost\Ubuntu\tmp\document.txt",
            r"\\?\UNC\files.example\share\document.txt",
            r"\\.\PhysicalDrive0",
            r"C:\",
            r"C:\scratch\asset.txt:notes",
            r"C:\scratch\..\asset.txt",
            r"C:\$Recycle.Bin\example\asset.txt",
        ] {
            assert!(recycle_path(Path::new(path)).is_err(), "accepted {path}");
        }
        assert!(recycle_path(Path::new(r"C:\scratch\café.txt")).is_ok());
    }

    struct Fixture {
        directory: PathBuf,
        file: PathBuf,
    }

    impl Fixture {
        fn create() -> Self {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = format!("monolith-recycle-{}-{stamp}", std::process::id());
            let directory = env::temp_dir().join(&id);
            // Fail on collision. Every later operation is on this newly created fixture.
            fs::create_dir(&directory).unwrap();
            Self {
                file: directory.join(format!("{id}-café.txt")),
                directory,
            }
        }

        fn write(&self, contents: &[u8]) {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&self.file)
                .unwrap();
            file.write_all(contents).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.file);
            let _ = fs::remove_dir(&self.directory);
        }
    }

    #[test]
    fn cancelled_selection_leaves_fixture_contents_intact() {
        let fixture = Fixture::create();
        fixture.write(b"untouched");
        let outcome = perform(std::slice::from_ref(&fixture.file), &AtomicBool::new(true)).unwrap();
        assert_eq!(outcome.recycled, 0);
        assert_eq!(outcome.error.unwrap().kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read(&fixture.file).unwrap(), b"untouched");
    }

    #[test]
    #[ignore = "requires an interactive Windows profile with an available Recycle Bin"]
    fn native_recycle_and_restore_unique_fixture() {
        let fixture = Fixture::create();
        fixture.write(b"recyclable");
        let _apartment = Apartment::new().unwrap();
        let bin = recycle_bin().unwrap();
        let mut volume = vec![0; 32_768];
        let (item, source) = recycle_source(&fixture.file, &mut volume).unwrap();
        let recycled = recycle_item(&source, &item, &bin, &AtomicBool::new(false)).unwrap();
        assert!(!fixture.file.exists());

        // Use only the returned object and our newly created original parent. A
        // collision cannot overwrite anything, and no bin enumeration is needed.
        let parent = unsafe { item.GetParent() }.unwrap();
        let restore: IFileOperation =
            unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER) }.unwrap();
        unsafe {
            restore
                .SetOperationFlags(
                    FOF_SILENT | FOF_NOERRORUI | FOFX_EARLYFAILURE | FOF_RENAMEONCOLLISION,
                )
                .unwrap();
            // The bin item is stored under an internal $R name; restore the original one.
            let name: Vec<u16> = fixture
                .file
                .file_name()
                .unwrap()
                .encode_wide()
                .chain([0])
                .collect();
            restore
                .MoveItem(&recycled, &parent, PCWSTR(name.as_ptr()), None)
                .unwrap();
            let result = restore.PerformOperations();
            assert!(!restore.GetAnyOperationsAborted().unwrap().as_bool());
            result.unwrap();
        }
        assert_eq!(fs::read(&fixture.file).unwrap(), b"recyclable");
    }
}
