# Contributing

Use the Windows MSVC toolchain for desktop changes. Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`. For UI changes, describe the interactive verification, Windows build and display scale used.

Document the behaviour and recovery path for new Windows integrations. Keep local settings, benchmark exports and workstation details out of commits.
