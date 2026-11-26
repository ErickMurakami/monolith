//! On-demand Git inspection in a read-only, single-process Windows job.

use std::{
    collections::{BTreeMap, HashMap},
    env,
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
        INVALID_HANDLE_VALUE, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Pipes::{CreatePipe, PeekNamedPipe},
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
            ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
};

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Eq, PartialEq)]
pub enum Branch {
    Named { name: String, unborn: bool },
    Detached { oid: String },
}

#[derive(Debug, Eq, PartialEq)]
pub struct Upstream {
    pub name: String,
    pub ahead_behind: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ItemStatus {
    pub staged: bool,
    pub modified: bool,
    pub untracked: bool,
    pub conflicted: bool,
}

pub struct Status {
    pub branch: Branch,
    pub upstream: Option<Upstream>,
    items: HashMap<OsString, ItemStatus>,
}

impl Status {
    pub fn item(&self, name: &OsStr) -> Option<ItemStatus> {
        self.items.get(name).copied()
    }
}

pub fn inspect(folder: &Path) -> io::Result<Option<Status>> {
    let git = installed_git()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Git is unavailable"))?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let (code, discovery) = query(
        &git,
        folder,
        "rev-parse --is-inside-work-tree --show-prefix",
        deadline,
    )?;
    if code != 0 {
        return if discovery.starts_with(b"fatal: not a git repository") {
            Ok(None)
        } else {
            Err(io::Error::other("Git could not discover this repository"))
        };
    }
    let end = discovery
        .iter()
        .position(|&byte| byte == b'\n')
        .ok_or_else(bad_output)?;
    let inside = &discovery[..end];
    match inside.strip_suffix(b"\r").unwrap_or(inside) {
        b"false" => return Ok(None),
        b"true" => (),
        _ => return Err(bad_output()),
    }
    let prefix = discovery[end + 1..]
        .strip_suffix(b"\n")
        .ok_or_else(bad_output)?;
    let prefix = prefix.strip_suffix(b"\r").unwrap_or(prefix);
    if prefix.contains(&b'\n')
        || prefix.contains(&0)
        || !prefix.is_empty() && !prefix.ends_with(b"/")
    {
        return Err(bad_output());
    }
    let (code, status) = query(
        &git,
        folder,
        "status --porcelain=v2 -z --branch --ahead-behind --no-renames --untracked-files=all --ignore-submodules=all -- .",
        deadline,
    )?;
    if code != 0 {
        return Err(io::Error::other(
            "Git could not read this repository's status",
        ));
    }
    parse(&status, prefix).map(Some)
}

fn installed_git() -> Option<PathBuf> {
    let search = env::var_os("PATH")?;
    for directory in env::split_paths(&search).filter(|path| path.is_absolute()) {
        let candidate = directory.join("git.exe");
        if !candidate.is_file() {
            continue;
        }
        // Git for Windows normally exposes a launcher in `cmd`. Its bundled CLI
        // can run inside the job without starting a second Git process.
        if directory.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("cmd") || name.eq_ignore_ascii_case("bin")
        }) && let Some(installation) = directory.parent()
        {
            for architecture in ["ucrt64", "mingw64", "clangarm64", "mingw32"] {
                let bundled = installation.join(architecture).join("bin").join("git.exe");
                if bundled.is_file() {
                    return Some(bundled);
                }
            }
        }
        return Some(candidate);
    }
    None
}

fn query(
    git: &Path,
    folder: &Path,
    arguments: &str,
    deadline: Instant,
) -> io::Result<(u32, Vec<u8>)> {
    if Instant::now() >= deadline {
        return Err(timed_out());
    }
    // Both outputs share the bounded pipe. Diagnostics on an otherwise successful
    // command invalidate the machine-readable result instead of hiding a problem.
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: 1,
        ..Default::default()
    };
    let mut read = null_mut();
    let mut write = null_mut();
    check(unsafe { CreatePipe(&mut read, &mut write, &security, 0) })?;
    let mut read = File::from(owned(read)?);
    let write = owned(write)?;
    check(unsafe { SetHandleInformation(read.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) })?;
    let discard = owned(unsafe {
        CreateFileW(
            windows_sys::core::w!("NUL"),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &security,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    })?;
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    })?;
    let handles = [write.as_raw_handle(), discard.as_raw_handle()];
    let jobs = [job.as_raw_handle()];
    let mut attributes = Attributes::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = discard.as_raw_handle();
    startup.StartupInfo.hStdOutput = write.as_raw_handle();
    startup.StartupInfo.hStdError = write.as_raw_handle();
    startup.lpAttributeList = attributes.as_ptr();

    let executable = wide(git.as_os_str());
    let folder = wide(folder.as_os_str());
    let environment = environment_block();
    let mut line = wide(OsStr::new(&format!(
        "git.exe --no-pager --no-optional-locks -c core.fsmonitor=false -c protocol.allow=never -c maintenance.auto=false {arguments}"
    )));
    let mut process = PROCESS_INFORMATION::default();
    check(unsafe {
        CreateProcessW(
            executable.as_ptr(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            folder.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    })?;
    let process_handle = owned(process.hProcess)?;
    let thread = owned(process.hThread)?;
    drop(write);
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    read_output(&mut read, &process_handle, deadline)
}

fn read_output(
    pipe: &mut File,
    process: &OwnedHandle,
    deadline: Instant,
) -> io::Result<(u32, Vec<u8>)> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut exited = false;
    loop {
        if Instant::now() >= deadline {
            return Err(timed_out());
        }
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_BROKEN_PIPE as i32) {
                return Err(error);
            }
        }
        if available != 0 {
            let length = (available as usize).min(buffer.len());
            let count = pipe.read(&mut buffer[..length])?;
            if output.len() + count > OUTPUT_LIMIT {
                return Err(io::Error::other("Git output exceeds its limit"));
            }
            output.extend_from_slice(&buffer[..count]);
            continue;
        }
        if exited {
            break;
        }
        match unsafe { WaitForSingleObject(process.as_raw_handle(), 15) } {
            WAIT_OBJECT_0 => exited = true,
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => {}
        }
    }
    let mut code = 0;
    check(unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) })?;
    Ok((code, output))
}

struct Attributes(Vec<usize>);

impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_usize; bytes.div_ceil(size_of::<usize>())];
        check(unsafe {
            InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 2, 0, &mut bytes)
        })?;
        Ok(Self(buffer))
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }

    fn set(&mut self, key: u32, handles: &[HANDLE]) -> io::Result<()> {
        check(unsafe {
            UpdateProcThreadAttribute(
                self.as_ptr(),
                0,
                key as usize,
                handles.as_ptr().cast(),
                size_of_val(handles),
                null_mut(),
                null(),
            )
        })
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn environment() -> BTreeMap<OsString, OsString> {
    let mut environment: BTreeMap<_, _> = env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            !key.starts_with("GIT_") && key != "LC_ALL"
        })
        .collect();
    for (key, value) in [
        ("GIT_CONFIG_GLOBAL", "NUL"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_ALLOW_PROTOCOL", ""),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("LC_ALL", "C"),
    ] {
        environment.insert(key.into(), value.into());
    }
    environment
}

fn environment_block() -> Vec<u16> {
    let mut entries: Vec<_> = environment().into_iter().collect();
    entries.sort_by_cached_key(|(key, _)| key.to_string_lossy().to_ascii_uppercase());
    let mut result = Vec::new();
    for (key, value) in entries {
        result.extend(key.encode_wide());
        result.push(b'=' as u16);
        result.extend(value.encode_wide());
        result.push(0);
    }
    result.push(0);
    result
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain([0]).collect()
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Git status took too long")
}

fn bad_output() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Git did not return a complete status",
    )
}

fn parse(output: &[u8], prefix: &[u8]) -> io::Result<Status> {
    let mut head = None;
    let mut oid = None;
    let mut upstream = None;
    let mut ahead_behind = None;
    let mut items = HashMap::<OsString, ItemStatus>::new();
    let terminated = output.strip_suffix(&[0]).ok_or_else(bad_output)?;
    for record in terminated.split(|&byte| byte == 0) {
        if let Some(header) = record.strip_prefix(b"# ") {
            if let Some(value) = header.strip_prefix(b"branch.head ") {
                head = Some(as_text(value)?);
            } else if let Some(value) = header.strip_prefix(b"branch.oid ") {
                oid = Some(as_text(value)?);
            } else if let Some(value) = header.strip_prefix(b"branch.upstream ") {
                upstream = Some(as_text(value)?);
            } else if let Some(value) = header.strip_prefix(b"branch.ab ") {
                let (ahead, behind) = as_text(value)?.split_once(' ').ok_or_else(bad_output)?;
                ahead_behind = Some((
                    ahead
                        .strip_prefix('+')
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(bad_output)?,
                    behind
                        .strip_prefix('-')
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(bad_output)?,
                ));
            }
            continue;
        }
        let (path, item) = if let Some(path) = record.strip_prefix(b"? ") {
            (
                path,
                ItemStatus {
                    untracked: true,
                    ..Default::default()
                },
            )
        } else if record.starts_with(b"u ") {
            let (_, path) = tracked(record, 11)?;
            (
                path,
                ItemStatus {
                    conflicted: true,
                    ..Default::default()
                },
            )
        } else if record.starts_with(b"1 ") {
            let (xy, path) = tracked(record, 9)?;
            (
                path,
                ItemStatus {
                    staged: xy[0] != b'.',
                    modified: xy[1] != b'.',
                    ..Default::default()
                },
            )
        } else {
            // This command disables rename detection and ignored records. Unknown
            // records here indicate diagnostics or an unsupported Git response.
            return Err(bad_output());
        };
        let Some(path) = path.strip_prefix(prefix) else {
            return Err(bad_output());
        };
        let name = path
            .split(|&byte| byte == b'/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or_else(bad_output)?;
        let combined = items.entry(OsString::from(as_text(name)?)).or_default();
        combined.staged |= item.staged;
        combined.modified |= item.modified;
        combined.untracked |= item.untracked;
        combined.conflicted |= item.conflicted;
    }
    let oid = oid.ok_or_else(bad_output)?;
    let branch = match head.ok_or_else(bad_output)? {
        "(detached)" => Branch::Detached { oid: oid.into() },
        name => Branch::Named {
            name: name.into(),
            unborn: oid == "(initial)",
        },
    };
    Ok(Status {
        branch,
        upstream: upstream.map(|name| Upstream {
            name: name.into(),
            ahead_behind,
        }),
        items,
    })
}

fn tracked(record: &[u8], width: usize) -> io::Result<(&[u8], &[u8])> {
    let mut columns = record.splitn(width, |&byte| byte == b' ');
    let xy = columns
        .nth(1)
        .filter(|xy| xy.len() == 2)
        .ok_or_else(bad_output)?;
    let path = columns
        .nth(width - 3)
        .filter(|path| !path.is_empty())
        .ok_or_else(bad_output)?;
    Ok((xy, path))
}

fn as_text(bytes: &[u8]) -> io::Result<&str> {
    str::from_utf8(bytes).map_err(|_| bad_output())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::process::CommandExt, process::Command, time::SystemTime};

    struct Repository {
        root: PathBuf,
        git: PathBuf,
    }

    impl Repository {
        fn create() -> Option<Self> {
            let Some(git) = installed_git() else {
                eprintln!("Git fixture skipped because Git is unavailable");
                return None;
            };
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "monolith-git-status-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            let repo = Self { root, git };
            assert!(
                inspect(&repo.root)
                    .unwrap_or_else(|error| {
                        let diagnostic = query(
                            &repo.git,
                            &repo.root,
                            "rev-parse --is-inside-work-tree --show-prefix",
                            Instant::now() + QUERY_TIMEOUT,
                        );
                        panic!("{error}: {diagnostic:?}");
                    })
                    .is_none()
            );
            repo.run(&["init", "--quiet", "--initial-branch=main", "--template="]);
            Some(repo)
        }

        fn command(&self, words: &[&str]) -> std::process::Output {
            Command::new(&self.git)
                .creation_flags(CREATE_NO_WINDOW)
                .env_clear()
                .envs(environment())
                .current_dir(&self.root)
                .args([
                    "-c",
                    "user.name=Monolith Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "maintenance.auto=false",
                ])
                .args(words)
                .output()
                .unwrap()
        }

        fn run(&self, words: &[&str]) {
            let output = self.command(words);
            assert!(
                output.status.success(),
                "{words:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            if let (Ok(directory), Ok(scratch)) = (
                fs::canonicalize(&self.root),
                fs::canonicalize(env::temp_dir()),
            ) && directory.parent() == Some(scratch.as_path())
                && directory.file_name() == self.root.file_name()
            {
                let _ = fs::remove_dir_all(directory);
            }
        }
    }

    #[test]
    fn reads_native_git_states_and_aggregates_directory_flags() {
        let Some(repo) = Repository::create() else {
            return;
        };
        let initial = inspect(&repo.root).unwrap().unwrap();
        assert_eq!(
            initial.branch,
            Branch::Named {
                name: "main".to_owned(),
                unborn: true
            }
        );
        repo.write("work/nested/áudio.txt", "original\n");
        repo.write("work/local.txt", "original\n");
        repo.write("conflict.txt", "original\n");
        repo.run(&["add", "."]);
        repo.run(&["commit", "--quiet", "-m", "Initial fixture"]);
        repo.run(&["branch", "divergent"]);
        repo.run(&["branch", "reference"]);
        repo.run(&["branch", "--set-upstream-to=reference", "main"]);
        repo.write("conflict.txt", "main\n");
        repo.run(&["commit", "--quiet", "-am", "Main fixture"]);
        repo.run(&["switch", "--quiet", "divergent"]);
        repo.write("conflict.txt", "divergent\n");
        repo.run(&["commit", "--quiet", "-am", "Divergent fixture"]);
        repo.run(&["switch", "--quiet", "main"]);
        assert!(!repo.command(&["merge", "divergent"]).status.success());
        repo.write("work/nested/áudio.txt", "staged\n");
        repo.run(&["add", "--", "work/nested/áudio.txt"]);
        repo.write("work/nested/áudio.txt", "modified since staging\n");
        repo.write("work/café.txt", "untracked\n");
        let index = fs::read(repo.root.join(".git/index")).unwrap();

        let root = inspect(&repo.root).unwrap().unwrap();
        assert_eq!(root.upstream.as_ref().unwrap().ahead_behind, Some((1, 0)));
        assert_eq!(
            root.item(OsStr::new("work")),
            Some(ItemStatus {
                staged: true,
                modified: true,
                untracked: true,
                conflicted: false
            })
        );
        assert!(root.item(OsStr::new("conflict.txt")).unwrap().conflicted);
        let folder = inspect(&repo.root.join("work")).unwrap().unwrap();
        assert!(folder.item(OsStr::new("café.txt")).unwrap().untracked);
        assert!(folder.item(OsStr::new("nested")).unwrap().staged);
        assert!(folder.item(OsStr::new("local.txt")).is_none());
        assert!(folder.item(OsStr::new("conflict.txt")).is_none());
        let differently_cased_folder = inspect(&repo.root.join("WORK")).unwrap().unwrap();
        assert!(
            differently_cased_folder
                .item(OsStr::new("café.txt"))
                .unwrap()
                .untracked
        );
        assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), index);

        repo.run(&["reset", "--hard", "HEAD"]);
        repo.run(&["switch", "--quiet", "--detach"]);
        assert!(
            matches!(inspect(&repo.root).unwrap().unwrap().branch, Branch::Detached { oid } if oid.len() == 40)
        );
        repo.run(&["worktree", "add", "--quiet", "--detach", "linked"]);
        assert!(matches!(
            inspect(&repo.root.join("linked/work")).unwrap().unwrap().branch,
            Branch::Detached { oid } if oid.len() == 40
        ));
    }

    #[test]
    fn git_cannot_start_a_repository_clean_filter() {
        let Some(repo) = Repository::create() else {
            return;
        };
        repo.write(".gitattributes", "danger.txt filter=external\n");
        repo.write("danger.txt", "original\n");
        repo.run(&["add", "."]);
        repo.run(&["commit", "--quiet", "-m", "Filter fixture"]);
        repo.run(&[
            "config",
            "filter.external.clean",
            "echo ran > filter-ran; cat",
        ]);
        repo.write("danger.txt", "modified\n");
        let ordinary = repo.command(&["--no-optional-locks", "status", "--porcelain"]);
        assert!(ordinary.status.success());
        assert!(
            repo.root.join("filter-ran").is_file(),
            "The reference Git command did not invoke the fixture filter"
        );
        fs::remove_file(repo.root.join("filter-ran")).unwrap();

        assert!(
            inspect(&repo.root).is_err(),
            "A denied clean filter must not present a possibly inaccurate result"
        );
        assert!(!repo.root.join("filter-ran").exists());
    }
}
