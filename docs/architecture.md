# Architecture

The Windows executable is a single Rust package. Modules separate desktop integration from testable logic.

- `config` parses local TOML and falls back to defaults when data cannot be read. Safe mode skips the read and suppresses shortcut registration.
- `search` discovers desktop shortcuts and ranks titles. `calculator` evaluates bounded arithmetic. Neither module depends on Win32.
- `desktop` reads top-level window titles and monitor geometry through documented Win32 and DWM APIs. It excludes hidden and cloaked windows, shell surfaces, auxiliary windows and Monolith itself. Fullscreen is inferred from window bounds in screen pixels. Window switching uses the normal foreground policy and validates the window handle and owning thread again when selected. Monitor numbers describe the current snapshot.
- Spotlight and Mission Control reuse the Dock's latest complete immutable window inventory when available. Explicit refresh reads a new snapshot. Window groups share the original records, and entries are revalidated before activation.
- `dock` owns its native window, child buttons and WinEvent subscriptions on the existing GUI thread. Out-of-context notifications are coalesced with a one-shot timer. A foreground location event can trigger a fullscreen recheck without continuous polling. Dock grouping uses the process application model ID for packaged apps and the executable path for desktop apps. Shared frame hosts and windows whose identity is inaccessible keep individual tiles. The Dock releases every hook before destroying its window.
- `top_bar` owns a separate status window on the same GUI thread. Native time and power notifications refresh the clock and battery data. A one-shot timer aligns clock changes to minute boundaries and stops when fullscreen hides the bar.
- `hotkey` owns native shortcut registration and uses Ctrl+Alt+Space when Windows already owns Win+Space. Safe mode skips global registration. Spotlight assigns foreground and edit focus before refreshing application and window results.
- `preview` links an on-demand, borderless Dock preview to a source window through DWM. Pointer departure, a tile click or an inventory update releases the thumbnail relationship. The preview has no capture loop and leaves foreground focus with the source application.
- `icons` loads Shell icons on a dedicated COM worker. A bounded queue limits requests to visible pins and application groups. The cache keeps raster data rather than native icon handles and converts to monochrome at paint time. Native button names and tooltips remain available.
- `media` subscribes to the Windows global media-session manager on a WinRT MTA worker. A bounded queue coalesces metadata and playback notifications. Each transport action is tied to a session generation so stale controls cannot operate on a newly selected session.
- `artwork` decodes session-provided images with Windows Imaging when the Island is expanded. The decode worker bounds input size and output dimensions. Session and metadata revisions reject results from a previous track or source.
- `island` renders the current media metadata and supported transport actions in a borderless, non-activating overlay. The Top Bar can host it as an owned window. Foreground WinEvents schedule fullscreen visibility changes. The expanded progress strip uses the reported position, update time and rate. Its one-second timer stops when paused, collapsed, hidden or at the timeline boundary.
- `control_center` opens an on-demand borderless panel with a native trackbar, keyboard navigation and fixed links to Windows Settings. Its `audio` worker subscribes to default render-endpoint and volume changes through Core Audio. Pending writes carry the output generation. Closing the panel releases the worker and its subscriptions.
- `mission` presents up to twelve live DWM thumbnails per page in a borderless overlay and releases the relationships when it closes. Previous and next actions invoke the documented Windows directional shortcuts from the foreground panel. The public `IVirtualDesktopManager` exposes a window's association. Windows Task View handles desktop enumeration, creation, naming, removal and reordering.
- `files` owns a separate, on-demand Win32 application window with custom title controls and an owner-data list view. `files_directory` runs shallow reads on one worker and discards stale results. It commits history only after successful reads, preserves original Windows filenames and bounds listings to 50,000 visible entries. Windows file association handlers run on explicit activation.
- `file_changes` creates folders and renames items on demand off the GUI thread. It validates a single destination name and uses native no-overwrite operations within the captured directory.
- `quick_look` opens an owned native preview window. `quick_content` reads up to 256 KiB of text or 16 MiB of image or PDF data and limits output to 1,024 pixels on the longest side. It uses Windows Imaging and Windows PDF rendering through memory streams. The preview closes on focus loss and releases content. Each Files process allows one active decode.
- `windows` owns the GUI thread, HWND lifetime, GDI resources and global shortcut. Explicit activation opens the selected shortcut through the Windows shell. Native messages drive painting, focus, layout and input.

The launcher is the first vertical slice. The application exits by default and has an explicit resident mode. No installer, service, login entry or Explorer integration is present in this checkpoint.

## Design

`overlay` defines shared borderless Win32 styles, palette and DPI geometry. Its dark theme uses #080A0D for the background, #141820 for controls and #E5E7EB for foreground text. The light theme uses #F1F3F6, #FFFFFF and #181C22. When Windows high contrast is enabled at launch, overlays read system colours. Spotlight retains native edit and list controls with the system selection highlight. Overlay controls use Segoe UI and opaque surfaces.

## Next product milestones

1. Extend validation of window enumeration to packaged apps and multiple display scales.
2. Extend the opt-in Dock with simultaneous per-display placement. Add the next Top Bar modules. Validate Explorer compatibility and recovery before adding taskbar visibility controls.
3. Add media sessions and workspace navigation through documented Windows APIs. Retain Task View for unsupported workspace operations.
4. Add Files, preview and an index. Validate file operation cancellation and recovery before writing through the UI.
5. Measure each workload before adding optimisations, settings and packaging.
6. Add opt-in Android continuity with authenticated pairing and local transport.
