# Monolith

Monolith is an experimental native desktop companion for Windows 11. The first build provides Spotlight, a local application launcher for Start menu shortcuts.

## Build and run

Use native Windows with the Rust MSVC toolchain and a Windows SDK.

```powershell
cargo build --locked --release
.\target\release\monolith.exe
```

Type an application name, use the arrow keys to select a match, and press Enter or double-click to open it. Escape closes the default session. Pass `--resident` to keep the launcher available after Escape when its global shortcut registered successfully. Close its window to quit. Use `--safe-mode` to start with defaults and the global shortcut disabled.

Monolith attempts to register Win+Space. Windows can reserve that combination. If it is unavailable, the window stays accessible and reports the conflict in its title. To change the binding, create `%LOCALAPPDATA%\Monolith\config.toml` and restart Monolith.

```toml
theme = "dark"              # dark, light
language = "english"        # english, portuguese
shortcut = "ctrl-alt-space" # win-space, ctrl-alt-space, disabled
```

Invalid settings are preserved on disk and defaults are loaded. Rename the file to restore defaults on the next start.

## Project status

- Implemented in the experimental build. Native Win32 launcher, local ranking of desktop shortcuts, configurable shortcut, light and dark themes, and safe start.
- Planned. Dock, Top Bar, media Island, workspaces, Files, indexing, optimisation, Android continuity, installer and release validation.
- Unsupported by the current launcher. UWP app discovery, file search, web search and arbitrary command execution.

The UI runs on the Win32 message queue and renders on demand. Spotlight builds its catalog on first use. Search and configuration are independent of Win32, with unit tests for ranking, discovery and recovery.

See [architecture](docs/architecture.md) for the implementation boundaries and roadmap.
