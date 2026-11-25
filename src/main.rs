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
mod top_bar;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
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
