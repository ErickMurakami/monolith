#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

#[cfg(windows)]
mod artwork;
#[cfg(windows)]
mod audio;
#[cfg(any(windows, test))]
mod calculator;
#[cfg(any(windows, test))]
mod config;
#[cfg(windows)]
mod control_center;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod dock;
#[cfg(windows)]
mod file_changes;
#[cfg(windows)]
mod file_clipboard;
#[cfg(windows)]
mod file_git;
#[cfg(windows)]
mod file_launch;
#[cfg(windows)]
mod file_recycle;
#[cfg(windows)]
mod file_transfer;
#[cfg(windows)]
mod files;
#[cfg(windows)]
mod files_directory;
#[cfg(windows)]
mod hotkey;
#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod island;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod mission;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod quick_content;
#[cfg(windows)]
mod quick_look;
#[cfg(any(windows, test))]
mod search;
#[cfg(windows)]
mod space_bridge;
#[cfg(windows)]
mod taskbar;
#[cfg(windows)]
mod top_bar;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == taskbar::HELPER_ARGUMENT)
    {
        std::process::exit(if taskbar::run_helper().is_ok() { 0 } else { 1 });
    }
    if let Err(error) = windows::run() {
        windows::report(&error.to_string());
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Monolith requires Windows 11.");
    std::process::exit(1);
}
