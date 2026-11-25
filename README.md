# Monolith

Monolith is an experimental native desktop companion for Windows 11. The first build provides Spotlight, a local application launcher for Start menu shortcuts.

## Build and run

Use native Windows with the Rust MSVC toolchain and a Windows SDK.

```powershell
cargo build --locked --release
.\target\release\monolith.exe
```

Spotlight opens as a compact, borderless overlay centred near the top of the active display. Type an application name, use the arrow keys to select a match, and press Enter or double-click to open it. Escape or a focus change dismisses Spotlight. Pass `--resident` to keep the launcher available after dismissal when its global shortcut registered successfully. Use `--safe-mode` to start with defaults and the global shortcut disabled.

Start a query with `=` to calculate locally. Try `=(12 + 3) / 2`. The calculator supports decimal numbers with a decimal point, parentheses, addition, subtraction, multiplication and division.

Start a query with `@` to find an open window by its title. Press Enter or double-click to switch to it, restoring it first if it is minimised. Press F5 to refresh the list. The window search uses desktop windows exposed by Windows, including packaged applications when they have an eligible desktop window.

Launch with `--dock` to show the experimental Dock. It floats along the bottom of the selected display and hides when a foreground window enters fullscreen there. Click a window to switch to it, or click its button again while it is active to minimise it. A group opens a native menu for choosing one of its windows. Hover over a running tile to preview its active or first visible window. **Search** opens Spotlight. **Exit** stops Monolith. The Dock shows up to eight pins and window groups depending on available space.

Launch with `--top-bar` to show a borderless status bar on the primary display. It displays the local clock and battery status when Windows reports a battery. Right-click the bar to open Spotlight or exit Monolith. It hides during fullscreen use.

Launch with `--island` to display the title and artist from the current Windows media session. Click the media label to expand play, pause, previous and next controls where the source supports them. Use `--island --top-bar` to place it in the centre of the status bar. The Island hides during fullscreen use and when Windows has no matching media session.

Monolith tries Win+Space first. If Windows reports that chord is already registered, Monolith registers Ctrl+Alt+Space as its fallback and displays it in the search placeholder. With `--resident`, the registered shortcut opens Spotlight and a second press dismisses it. Escape returns focus to the previous window when Windows permits it. To change the binding, create `%LOCALAPPDATA%\Monolith\config.toml` and restart Monolith.

```toml
theme = "dark"              # dark, light
language = "english"        # english, portuguese
shortcut = "ctrl-alt-space" # win-space, ctrl-alt-space, disabled
dock = false               # set true to opt in
dock_pins = ["Visual Studio Code"] # exact Start menu shortcut names, in display order
dock_monitor = "primary"           # primary, active
dock_auto_hide = false             # reveal at the bottom of the work area
dock_original_icons = false       # keep the original Shell colours when true
top_bar = false            # set true to opt in
island = false             # set true to opt in
```

Pins are resolved from local Start menu shortcuts. Missing and repeated names are skipped. Pins take the first available slots in the Dock. Drag a pin onto another pin to reorder it. The order is saved in `dock-order.toml` next to the configuration and takes precedence over the list. Remove the order file to restore configuration order. Windows Shell icons are monochrome by default, with names available through native tooltips and accessibility. A tile uses its text label when the Shell cannot supply an icon.

Invalid settings are preserved on disk and defaults are loaded. Rename the file to restore defaults on the next start.

## Project status

- Implemented in the experimental build. Native Win32 launcher, local ranking of desktop shortcuts and open windows, calculator, opt-in running-window Dock, configurable shortcut, light and dark themes, and safe start.
- Planned. Simultaneous per-display Docks, additional Top Bar modules, Island artwork and progress, workspaces, Files, indexing, optimisation, Android continuity, installer and release validation.
- Unsupported by the current launcher. UWP app discovery, file search, web search and arbitrary command execution.

The UI runs on the Win32 message queue and renders on demand. Spotlight builds its catalog on first use. Search and configuration are independent of Win32, with unit tests for ranking, discovery and recovery.

See [architecture](docs/architecture.md) for the implementation boundaries and roadmap.

See [project status](MONOLITH_STATUS.md) and [benchmark procedure](benchmarks/README.md).

![Borderless Spotlight showing a calculation on Windows](docs/spotlight.png)

![Cropped Dock capture showing the monochrome Shell icon for a verification shortcut](docs/dock.png)

![Cropped Top Bar capture showing system power and clock](docs/top-bar.png)

![Dynamic Island showing a live local media session and transport controls](docs/island.png)
