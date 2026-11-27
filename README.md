# Monolith

Monolith is an experimental native desktop companion for Windows 11. The first build provides Spotlight, a local application launcher for Start menu shortcuts.

## Build and run

Use native Windows with the Rust MSVC toolchain and a Windows SDK.

```powershell
cargo build --locked --release
.\target\release\monolith.exe
```

Spotlight opens as a compact, borderless overlay centred near the top of the active display. Type an application name, use the arrow keys to select a match, and press Enter or double-click to open it. Escape or a focus change dismisses Spotlight. Pass `--resident` to keep the launcher available after dismissal when its global shortcut registered successfully. Use `--safe-mode` to start with defaults and the global shortcut disabled. Press Ctrl+Enter to open the selected application in a new Windows desktop instead; Monolith creates the desktop with the native Win+Ctrl+D shortcut and launches the application once Explorer reports the switch. An application that is already running may bring its existing window back instead. When you close the last window on that desktop, Monolith closes the now-empty desktop and returns you to the one you started from. Desktops Monolith did not create are never closed. Spotlight also lists applications from the Windows Apps folder that have no Start menu shortcut, such as Windows Terminal, Settings and Store apps.

Start a query with `=` to calculate locally. Try `=(12 + 3) / 2`. The calculator supports decimal numbers with a decimal point, parentheses, addition, subtraction, multiplication and division.

Start a query with `@` to find an open window by its title. Press Enter or double-click to switch to it, restoring it first if it is minimised. Press F5 to refresh the list. The window search uses desktop windows exposed by Windows, including packaged applications when they have an eligible desktop window.

Launch with `--dock` to show the experimental Dock. It floats along the bottom of the selected display and hides when a foreground window enters fullscreen there. Click a window to switch to it, or click its button again while it is active to minimise it. A group opens a native menu for choosing one of its windows. Hover over a running tile to preview its active or first visible window. **Search** opens Spotlight. **Exit** stops Monolith. The Dock shows up to eight pins and window groups depending on available space.

Launch with `--replace-taskbar` to try the Dock in the existing Windows taskbar band. The option requires one primary display and a bottom Explorer taskbar with native auto-hide off. Monolith preserves the Windows work area and taskbar preference. **Exit** or a separate `--safe-mode` launch releases the taskbar. A recovery process handles unexpected termination of the Dock, and the Dock restores the taskbar if the recovery process terminates. Simultaneous termination of both processes can require restarting Explorer to restore visibility.

Launch with `--top-bar` to show a borderless status bar on the primary display. Windows reserves space for the bar so maximised windows keep their captions accessible. It displays the local date and time, plus battery status when available. Right-click the bar to open Spotlight, Control Center or exit Monolith. The bar hides during fullscreen use or when a maximised foreground window extends into the strip despite the reservation. The sliders and bell at its right end open Control Center and the Notification Centre; a blue dot on the bell marks unread notifications.

Launch with `--island` to display the title and artist from the current Windows media session. Click the media label to expand the artwork and play, pause, previous and next controls where the source provides them. A progress strip appears when Windows reports a valid timeline. Use `--island --top-bar` to place it in the centre of the status bar. The Island hides during fullscreen use and when Windows has no matching media session. When expanded, the Island also shows the album, elapsed and total time, and shuffle and repeat when the session supports them. Click the progress bar to seek.

Launch with `--control-center` or use the Top Bar menu to open a borderless panel. It adjusts volume and mute for the Windows default audio output and shows the battery percentage when available. The settings buttons open the corresponding Windows pages for Wi-Fi, Bluetooth, display, night light, sound devices, focus and power. Use Tab and the arrow keys to navigate and adjust the volume. Escape or a focus change closes the panel. On a laptop panel that reports brightness through WMI, a brightness slider changes the display brightness directly; external monitors do not show it. The Wi-Fi and Bluetooth buttons show the radio state and switch the radio directly; when Windows reports a radio as unavailable, they open its Settings page instead.

Launch with `--mission-control` or select **Mission Control** from the Dock or Top Bar context menu. The full-screen overview follows the macOS layout: a strip of your Windows desktops across the top (the current one outlined in blue), then a card per desktop with live previews of up to four of its windows, including windows on other desktops. A desktop holding a single fullscreen app is named after it. Click a desktop's pill or card, or press 1 to 9, to switch to it; click a window to switch to its desktop and focus it. The arrows below the cards, or Page Up and Page Down, move to the adjacent desktop, and clicking outside the cards closes the overview. **+** adds a desktop and the Task View button opens the original Windows desktop manager for arranging and moving windows. F5 refreshes the inventory.

Launch with `--files` or select **Files** from a shell context menu. Pass a folder after `--files` to start there. The native window provides a virtualised list, shortcuts to standard folders and a location field. Use Ctrl+L for the location field, Alt+Left and Alt+Right for history, Alt+Up for the parent folder, Ctrl+H for hidden files and F5 to refresh. Open a selected folder or file with Enter or a double-click. File activation uses the Windows default application. Press F2 to rename an item or Ctrl+Shift+N to create a folder. Both actions are available from the Files context menu and reject existing destination names.

With the file list focused, use Ctrl+C, Ctrl+X and Ctrl+V to copy, cut and paste. The context menu provides the same actions. The Windows Shell keeps existing destination items by naming incoming copies or moves differently when needed. Press Escape in Files or use the Windows transfer dialog to request cancellation. A batch interrupted after some items have transferred can leave those completed items in the destination.

Press Delete, or choose **Move to Recycle Bin** from the context menu, to recycle the focused or selected items on fixed local volumes. Files never deletes permanently: Shift+Delete does nothing, and items the Recycle Bin rejects, such as items on network or WSL paths and reparse points, stay in place with an explanation. Restore recycled items from the Windows Recycle Bin.

Press Space on a file or folder in the list to open Quick Look. It shows common image formats, the first PDF page, UTF-8 or UTF-16 text and basic item details. Markdown and source files are displayed as literal text. Press Space or Escape to return to the list.

Select **Show Git** or press Ctrl+G to see the current branch, locally known upstream counts, and staged, modified, untracked or conflicted items. Folder rows summarise changes within them. The view uses an installed Git for Windows and refreshes on navigation or F5. Git status starts off in each Files window.

Use **Open terminal here** in the Files context menu to start a terminal in the displayed directory. Monolith uses Windows Terminal when its alias is available and uses Windows PowerShell as the fallback. Set `files_ide` to the absolute path of a local IDE executable to show **Open in configured IDE**. It opens the selected folder, or the displayed directory when a folder is unavailable in the selection.

Monolith tries Win+Space first and uses Ctrl+Alt+Space as a fallback when Windows reserves the preferred chord. Set `win_space_override = true` (experimental) to keep Win+Space working anyway: a keyboard hook on its own thread sends the registered fallback chord when you press Win+Space and replays anything typed before Spotlight has focus, so the first characters are not lost. The hook is removed when Monolith exits, and Win+Space returns to Windows (input language switching). With `--resident`, pressing the registered shortcut again dismisses Spotlight. Escape returns focus to the previous window when Windows permits it. Safe mode disables global shortcuts.
Set `workspace_shortcuts = true` (experimental) to let Win+1 through Win+9 select native Windows desktops by their Task View order and Win+Tab open or close Mission Control. Monolith reads the desktop order from Explorer and switches with the native Win+Ctrl+Left/Right shortcuts once you release the Windows key, so desktops stay owned by Windows. Windows keeps Win+Shift+number and Task View remains available from Mission Control.
Set `desktop_moves = true` (experimental) to let Win+Shift+1 through Win+Shift+9 move the focused window to that desktop without switching your view. Windows offers no public way to move another application's window, so this uses Explorer's undocumented virtual desktop interfaces through an isolated adapter. It runs only on Windows builds 26100 to 26999, only after confirming that its view of desktops matches Explorer's, and it switches itself off otherwise. Nothing is injected into Explorer. Set `fullscreen_workspaces = true` (experimental) to give an application that enters fullscreen its own desktop, through the same adapter. When it leaves fullscreen or closes, the window returns, the extra desktop is removed and you are back where you were.
Set `touchpad_gestures = true` (experimental) for macOS-style three-finger swipes on a precision touchpad: swipe left or right to move to the neighbouring desktop, up to open Mission Control and down to show the desktop (as Win+D). Monolith reads the touchpad through Windows Raw Input; it installs no driver and changes no settings. Set the Windows three-finger gestures to **Nothing** (Settings > Bluetooth & devices > Touchpad > Three-finger gestures) so Windows does not act on the same swipe.
Set `notifications = true`, or pass `--notifications` (experimental), to show incoming Windows notifications in a compact banner below the Island, with the app icon, name, title and body. The banner never takes focus, stays hidden over fullscreen windows and disappears after six seconds. Clicking it brings up an open window of the source app when Monolith can identify one: packaged apps, desktop apps that set an application ID on their windows, and desktop apps identified by their executable path. Windows still shows and keeps its own notification, so nothing is lost if Monolith stops. Do Not Disturb silences both. Click the bell at the right end of the Top Bar, or choose **Notifications** from the Dock or Top Bar menu, to open the Notification Centre at the upper right. It lists this session's notifications, newest first, with unread markers. Click an entry to dismiss it from Monolith, or **Clear all**. History is kept in memory only, capped at 50 entries, and Windows keeps its own copies. Set `hide_notification_content = true` to show and keep only the source app, never the text. Allow notification access in Windows Settings (Privacy & security > Notifications) if the banner never appears.

Monolith reads `%LOCALAPPDATA%\Monolith\config.toml` at startup.

```toml
theme = "dark"              # dark, light
language = "english"        # english, portuguese
# files_ide = 'C:\Tools\IDE.exe'
shortcut = "ctrl-alt-space" # win-space, ctrl-alt-space, disabled
# win_space_override = true # experimental Win+Space bridge when Windows reserves the chord
# workspace_shortcuts = true # experimental Win+1-9 desktop selection and Win+Tab Mission Control
# desktop_moves = true       # experimental Win+Shift+1-9 moves the focused window to a desktop
# fullscreen_workspaces = true # experimental: fullscreen apps get a desktop of their own
# close_empty_workspaces = true # experimental: close empty desktops, keeping one
# touchpad_gestures = true   # experimental: three-finger swipes switch desktops, open Mission Control, show the desktop
# notifications = true      # experimental incoming notifications below the Island
# hide_notification_content = true # show only the source app, never the text
dock = false               # set true to opt in
dock_pins = ["Visual Studio Code"] # exact Start menu shortcut names, in display order
dock_monitor = "primary"           # primary, active
dock_auto_hide = false             # reveal at the bottom of the work area
dock_original_icons = false       # keep the original Shell colours when true
top_bar = false            # set true to opt in
island = false             # set true to opt in
```

Pins are resolved from local Start menu shortcuts. Missing and repeated names are skipped. Pins take the first available slots in the Dock. Drag a pin onto another pin to reorder it. The order is saved in `dock-order.toml` next to the configuration and takes precedence over the list. Remove the order file to restore configuration order. Windows Shell icons are monochrome by default, with names available through native tooltips and accessibility. A tile uses its text label when the Shell cannot supply an icon. A pin whose shortcut directly targets an executable adopts that application's running windows: clicking it switches to, minimises or chooses among them instead of starting another instance, and a small blue indicator marks it as running. Launcher stubs, packaged apps and shortcuts to documents keep their pin as a launcher beside a separate running tile. Shift+click a pin to start another instance.

Invalid settings are preserved on disk and defaults are loaded. Rename the file to restore defaults on the next start.

## Project status

- Implemented in the experimental build. Native Win32 launcher, local ranking of desktop shortcuts and open windows, calculator, opt-in running-window Dock, configurable shortcut, light and dark themes, and safe start.
- Planned. Simultaneous per-display Docks, additional Top Bar modules, media and Android integrations for Control Center, workspaces, Files layout and operations, indexing, optimisation, installer and release validation.
- Unsupported by the current launcher. UWP app discovery, file search, web search and arbitrary command execution.

The UI runs on the Win32 message queue and renders on demand. Spotlight builds its catalog on first use. Search and configuration are independent of Win32, with unit tests for ranking, discovery and recovery.

See [architecture](docs/architecture.md) for the implementation boundaries and roadmap.

See [project status](MONOLITH_STATUS.md) and [benchmark procedure](benchmarks/README.md).

![Borderless Spotlight showing a calculation on Windows](docs/spotlight.png)

![Live Dock centred in the Windows taskbar band](docs/dock.png)

![Cropped Top Bar capture showing battery, date and time](docs/top-bar.png)

![Dynamic Island showing a live local media session and transport controls](docs/island.png)

![Control Center with native volume and links to Windows settings](docs/control-center.png)

![Two live window thumbnails in the Mission Control grid](docs/mission.png)

![Monolith Files showing a verification folder and the standard folder shortcuts](docs/files.png)

![Quick Look rendering a local PNG in the native preview window](docs/quick-look.png)

![Files showing the branch and item states for a verification repository](docs/files-git.png)
