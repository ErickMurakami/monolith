# Architecture

The Windows executable is a single Rust package. Modules separate desktop integration from testable logic.

- `config` parses local TOML and falls back to defaults when data cannot be read. Safe mode skips the read and suppresses shortcut registration.
- `search` discovers desktop shortcuts and ranks titles. `calculator` evaluates bounded arithmetic. Neither module depends on Win32.
- `desktop` reads top-level window titles and monitor geometry through documented Win32 and DWM APIs. It excludes hidden and cloaked windows, shell surfaces, auxiliary windows and Monolith itself. Fullscreen is inferred from window bounds in screen pixels. Window switching uses the normal foreground policy and validates the window handle and owning thread again when selected. Monitor numbers describe the current snapshot.
- `windows` owns the GUI thread, HWND lifetime, GDI resources and global shortcut. Explicit activation opens the selected shortcut through the Windows shell. Native messages drive painting, focus, layout and input.

The launcher is the first vertical slice. The application exits by default and has an explicit resident mode. No installer, service, login entry or Explorer integration is present in this checkpoint.

## Design

The dark theme uses #080A0D for the background, #141820 for controls and #E5E7EB for foreground text. The light theme uses #F1F3F6, #FFFFFF and #181C22. The system draws the non-client frame and selection highlight, respecting its native colour choices. Controls use Segoe UI. Layout spacing scales with the monitor DPI context. The current interface uses opaque surfaces and has no animation loop.

## Next product milestones

1. Extend validation of window enumeration to packaged apps and multiple display scales.
2. Add Dock and Top Bar as independently disabled surfaces. Validate foreground changes, process exit and Explorer compatibility before adding taskbar visibility controls.
3. Add media sessions and workspace navigation through documented Windows APIs. Retain Task View for unsupported workspace operations.
4. Add Files, preview and an index. Validate file operation cancellation and recovery before writing through the UI.
5. Measure each workload before adding optimisations, settings and packaging.
6. Add opt-in Android continuity with authenticated pairing and local transport.
