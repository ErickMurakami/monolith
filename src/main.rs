#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

#[cfg(any(windows, test))]
mod calculator;
#[cfg(any(windows, test))]
mod config;
#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod dock;
#[cfg(any(windows, test))]
mod search;
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
