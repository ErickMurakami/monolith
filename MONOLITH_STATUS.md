# Monolith status

## Current milestone

Foundation and the first native application are implemented. Desktop Experience work has delivered the experimental Spotlight application launcher, calculator, window switcher and opt-in Dock.

- `src/windows.rs` owns the borderless Spotlight window lifecycle, keyboard navigation, focus dismissal and shortcut registration.
- `src/overlay.rs` shares the native overlay styles, palette and DPI scaling between Spotlight and Dock.
- `src/hotkey.rs` owns `RegisterHotKey` registration and switches to Ctrl+Alt+Space when Windows reserves Win+Space.
- `src/hotkey_override.rs` supports an opt-in Win+Space override that activates Spotlight through the registered chord after logo-key release.
- `src/search.rs` discovers Start menu desktop shortcuts and ranks local matches.
- `src/calculator.rs` evaluates bounded arithmetic expressions entered in Spotlight.
- `src/desktop.rs` discovers eligible windows and their display, detects fullscreen geometry and switches on explicit selection.
- `src/dock.rs` presents ordered, configurable Start menu pins and open windows on the primary display. The Dock handles live updates, minimisation, restoration and fullscreen suppression.
- `src/config.rs` validates TOML, preserves invalid files and supports a safe start.
- `src/top_bar.rs` displays native clock and battery data in an opt-in borderless overlay.
- `src/preview.rs` owns the on-demand DWM thumbnail shown when hovering a running Dock tile.
- `src/icons.rs` extracts authentic Shell icons on a bounded worker and renders monochrome or original colours.
- `src/media.rs` subscribes to Windows media sessions and handles supported transport actions on a bounded WinRT worker.
- `src/artwork.rs` decodes current session artwork on demand with bounded Windows Imaging streams.
- `src/island.rs` presents real media metadata, controls and timeline progress in a borderless optional overlay.
- `src/control_center.rs` adjusts the default audio output, displays available battery data and opens documented Windows Settings pages. `src/audio.rs` bounds updates and releases Core Audio subscriptions when the panel closes.
- `src/mission.rs` displays a native borderless window overview with live DWM thumbnails, the shared Dock inventory, paging, keyboard and mouse selection and a Windows Task View action.
- `src/files.rs` owns a native, on-demand Files window with a custom title surface, virtualised navigation, standard Windows folders and default app activation. `src/files_directory.rs` loads directories off the GUI thread and preserves history on failures.
- `src/quick_look.rs` opens a borderless text, image, PDF or metadata preview on Space in Files. `src/quick_content.rs` bounds reads and pixel counts and uses platform decoders.
- `src/file_changes.rs` performs same-directory rename and folder creation without overwriting an existing destination.
- `benchmarks/measure.ps1` samples Windows and Monolith memory, CPU and I/O.
- The benchmark also records the busiest GPU engine from WDDM counters when available.
- `docs/spotlight.png` shows a calculation in the running borderless Spotlight release build.
- `docs/dock.png` is a cropped capture of a monochrome icon in the running Dock.
- `docs/top-bar.png` is a cropped live capture of the Top Bar status area.
- `docs/island.png` shows the Island controlling a real Helium media session carrying local test audio.
- `docs/control-center.png` shows the live borderless panel and the Windows settings actions.
- `docs/mission.png` shows a cropped capture of two real DWM thumbnails for verification windows.
- `docs/files.png` shows the real directory list and standard-folder actions with verification files.
- `docs/quick-look.png` shows the native preview rendering a verification PNG.

The Dock groups running windows by packaged application ID or verified process image, opens a native window chooser and previews the active or first visible window in a group. It can follow the active display, retract to a pointer reveal strip and show native Shell icons for pins and running groups. The Island shows title, artist, artwork, available transport controls and progress from the Windows timeline. Simultaneous per-display Docks, additional Top Bar modules, media and Android integrations for Control Center, workspaces, Files previews and operations, indexing, tuning, settings UI, Android integration and installers are pending.

## Validation

Validated on native Windows 11 26H2. Cargo formatting, Clippy with warnings denied, seventeen unit tests and the locked release build passed. A live desktop run verified shortcut launch, safe-mode exit and calculator output. `tests/desktop-smoke.ps1` verified filtering out hidden and owned windows, identifying fullscreen and minimised windows, refreshing after a window closes and switching to a minimised window. The same script with `-Dock` verified mouse clicks, minimisation, restoration, title changes, grouped menu selection, fullscreen suppression and recovery, Spotlight opening, clean exit and safe-mode suppression. The script with `-Pins` verified configured ordering, drag persistence after restart, deduplication, launching a real shortcut, rendered monochrome icon pixels and a native tooltip. An additional live check found packaged Notepad by the title of a temporary test document. The `-Spotlight` test verified the borderless styles, placement, calculation, Escape and focus-loss dismissal. The `-TopBar` test verified the clock, non-activating styles, fullscreen hiding, Spotlight and Exit. The `-AutoHide` test verified active-display placement, pointer reveal and pointer retraction on one attached monitor. The `-Preview` test verified actual DWM thumbnail pixels, focus preservation and cleanup on pointer departure. Live Spotlight, Dock and Top Bar captures were visually reviewed. Cross-build, high contrast and mixed-DPI testing remain pending.

The media smoke test used an isolated local Helium profile and real Windows media-session events. It verified expansion, title and artist, artwork pixels from the session, play, pause, next, previous, progress, paused timeline stability and fullscreen recovery. The Island artwork capture was visually reviewed. The artwork checkpoint passed eighteen unit tests.

`tests/control-center-smoke.ps1` verified borderless styles, keyboard navigation, actual Core Audio level and mute changes, external notifications, foreground restoration, focus-loss dismissal, reopening from the Top Bar and opening Windows Settings. It restored the initial audio level and mute state. The live panel capture was visually reviewed. Mixed-DPI displays and physical output switching remain to be tested.

`tests/mission-smoke.ps1` verified borderless styles, live thumbnail pixels and updates, mouse switching, keyboard restoration of a minimised window, F5 after a source closes, the Dock and Top Bar menu entries and native Task View. Nineteen unit tests passed. Windows Task View remains the path for virtual desktop thumbnails, creation, naming, reordering and moves. Mixed-DPI and multiple physical display validation remain pending.

`tests/files-smoke.ps1` verified the virtualised list, custom minimise, maximise, restore and close, folder history, parent navigation, Unicode paths, hidden files, error recovery, refresh and launch from the shell. Twenty-one unit tests passed. The Files capture was visually reviewed. Live removable, UNC and WSL volume tests remain pending.

`tests/quick-look-smoke.ps1` verified folder details, literal Markdown, genuine PNG and PDF pixels, handling of malformed and oversized inputs, keyboard dismissal and shutdown with an open preview. Twenty-two unit tests passed. The live PNG capture was visually reviewed.

On Windows 11 26H2 build 26300.9974, `tests/files-change-smoke.ps1` verified the Files changes in `3e98417`, including rejection of a reserved name, preservation of the existing file during a conflict, Unicode and case-only rename, folder creation, and cancellation. `tests/workspace-navigation-smoke.ps1` verified directional switching through native desktop membership, a boundary without wrap and restoration of the original desktop.

Checked the declared licences of all locked dependencies, including the added Windows Runtime bindings. Cargo Audit passed with warnings denied across 31 lockfile entries.

## Measurements

Six samples at nominal two-second intervals per scenario. Windows CPU was 74-100% during baseline sampling and 94-100% with the launcher visible while development tools were compiling. The launcher recorded 1.36-1.39 MiB of private working set, 13.13-13.17 MiB of total working set and 1.86-1.91 MiB of private bytes. All six launcher CPU and I/O counter readings were zero at the sampled resolution. These results describe this run. Repeat matched baseline, visible and resident scenarios after background work settles before drawing system-wide conclusions.

The Dock was sampled in two baseline/Dock pairs with six samples per scenario at nominal one-second intervals. The Dock was visible with Spotlight hidden. Across both runs, the process used 1.00-1.02 MiB of private working set, 9.43-9.45 MiB of total working set and 1.45-1.59 MiB of private bytes. Sampled Monolith CPU and I/O counters were zero. Baseline system CPU ranged from 0% to 35%, and Dock system CPU from 0% to 49%. This short measurement does not establish a system-wide CPU impact.

After the borderless Spotlight and pin-reordering changes, two matched idle runs provided twelve samples per scenario. The Dock alone used 1.11-1.13 MiB of private working set, 9.90-9.93 MiB of total working set and 1.60-1.74 MiB of private bytes. Standalone Spotlight used 1.37-1.41 MiB, 13.33-13.36 MiB and 1.88-1.91 MiB respectively. Monolith CPU, GPU and I/O counters read zero at sample resolution. System busiest-engine GPU ranges were 1-6% at baseline, 2-4% with the Dock and 1-3% with Spotlight. System CPU ranges overlapped across the scenarios.

After adding Shell icons, twelve idle Dock samples across two runs measured 2.05-2.11 MiB of private working set, 19.36-19.41 MiB of total working set and 4.37-4.43 MiB of private bytes. The worker had loaded visible tiles and no preview was open. App CPU, GPU and I/O counters read zero at sample resolution. System busiest-engine GPU ranged from 1% to 15% at baseline and 1% to 4% during Dock sampling. The system values are workload observations.

At `6fcb723`, two baseline and shell-idle pairs provided twelve samples per scenario. The Dock and Top Bar were visible, Spotlight was closed and the enabled Island had no active media presentation. The combined process used 2.33-2.36 MiB of private working set, 25.27-25.31 MiB of total working set and 5.03-5.07 MiB of private bytes. App CPU, busiest GPU engine and I/O counters read zero at sample resolution. System CPU ranged from 0% to 51% at baseline and 0% to 49% with the shell. System busiest-engine GPU ranged from 1% to 4% at baseline and 1% to 5% with the shell.

At `c04bb81`, two matched idle pairs provided twelve samples per scenario with the Dock and Top Bar visible, the Island enabled with no active presentation and Spotlight, Mission Control and Control Center closed. Monolith used 2.38-2.41 MiB of private working set, 25.37-25.41 MiB of total working set and 5.09-5.13 MiB of private bytes. Its sampled CPU, busiest GPU engine and I/O counters read zero. System CPU ranged from 0% to 38% at baseline and 0% to 68% with the shell. System busiest-engine GPU ranged from 1% to 4% and 1% to 5%, respectively.

## Git and recovery

The launcher, benchmark, calculator and window-switcher checkpoints are `e86dc9a`, `5f279bb`, `9b9b555` and `a8cef7b`. The project uses `main` and an SSH remote under the verified personal account. The Island media-session checkpoint `d8081a1` was fast-forwarded into `main`. Its merged development branch was removed locally. Workstation-specific records, configuration and raw benchmark exports stay local.

The binary needs no elevation and makes no persistent changes to Windows. Dismiss standalone Spotlight or use **Exit** on the Dock to stop the process and release native hooks. Pass `--safe-mode` to bypass configuration and start Spotlight alone. Rename the configuration file to recover defaults. Portable removal consists of closing the process and removing the executable, with optional removal of the local configuration directory.

The Windows CI checkpoint `4780b92` built and uploaded `monolith.exe`. Run `45824821038` verified locked x64 validation and an AMD64 executable with seven-day artifact retention. The release job skipped the branch push.

## Shortcut compatibility

The temporary Ctrl+Alt+Space path still passes `-Hotkey`. With `win_space_override = true`, `tests/spotlight-override-smoke.ps1` passed twenty native trials on build 26300.9974. It exercised both Windows keys, normal, maximised and fullscreen windows in another process, first-key delivery after a requested 15 ms delay, repeat suppression, focus restoration, safe mode and an unchanged keyboard layout. The observer saw no top-level system input or Start window during the trials. Physical keyboard trials and elevated foreground behaviour remain unverified. The normal startup and safe-mode paths keep the override disabled by default.

## Next task

Validate combined resident execution with Win+Space enabled. Stable workspace identity, dedicated application placement, physical gestures and crash-safe taskbar restoration remain open. Wire the bounded read-only Git status backend into Files while those integrations are investigated.
