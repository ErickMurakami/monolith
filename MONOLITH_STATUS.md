# Monolith status

## Current milestone

Foundation and the first native application are implemented. Desktop Experience work has delivered the experimental Spotlight application launcher, calculator, window switcher and opt-in Dock.

- `src/windows.rs` owns the borderless Spotlight window lifecycle, keyboard navigation, focus dismissal and shortcut registration.
- `src/overlay.rs` shares the native overlay styles, palette and DPI scaling between Spotlight and Dock.
- `src/hotkey.rs` owns `RegisterHotKey` registration and switches to Ctrl+Alt+Space when Windows reserves Win+Space.
- `src/search.rs` discovers Start menu desktop shortcuts and ranks local matches.
- `src/calculator.rs` evaluates bounded arithmetic expressions entered in Spotlight.
- `src/desktop.rs` discovers eligible windows and their display, detects fullscreen geometry and switches on explicit selection.
- `src/dock.rs` presents ordered, configurable Start menu pins and open windows on the primary display. The Dock handles live updates, minimisation, restoration and fullscreen suppression.
- `src/config.rs` validates TOML, preserves invalid files and supports a safe start.
- `src/top_bar.rs` displays native clock and battery data in an opt-in borderless overlay.
- `src/preview.rs` owns the on-demand DWM thumbnail shown when hovering a running Dock tile.
- `src/icons.rs` extracts authentic Shell icons on a bounded worker and renders monochrome or original colours.
- `src/media.rs` subscribes to Windows media sessions and handles supported transport actions on a bounded WinRT worker.
- `src/island.rs` presents real media metadata and controls in a borderless optional overlay.
- `benchmarks/measure.ps1` samples Windows and Monolith memory, CPU and I/O.
- The benchmark also records the busiest GPU engine from WDDM counters when available.
- `docs/spotlight.png` shows a calculation in the running borderless Spotlight release build.
- `docs/dock.png` is a cropped capture of a monochrome icon in the running Dock.
- `docs/top-bar.png` is a cropped live capture of the Top Bar status area.
- `docs/island.png` shows the Island controlling a real Helium media session carrying local test audio.

The Dock groups running windows by packaged application ID or verified process image, opens a native window chooser and previews the active or first visible window in a group. It can follow the active display, retract to a pointer reveal strip and show native Shell icons for pins and running groups. The Island shows title, artist and available transport controls. Simultaneous per-display Docks, additional Top Bar modules, Island artwork and progress, workspaces, Files, indexing, tuning, settings UI, Android integration and installers are pending.

## Validation

Validated on native Windows 11 26H2. Cargo formatting, Clippy with warnings denied, sixteen unit tests and the locked release build passed. A live desktop run verified shortcut launch, safe-mode exit and calculator output. `tests/desktop-smoke.ps1` verified filtering out hidden and owned windows, identifying fullscreen and minimised windows, refreshing after a window closes and switching to a minimised window. The same script with `-Dock` verified mouse clicks, minimisation, restoration, title changes, grouped menu selection, fullscreen suppression and recovery, Spotlight opening, clean exit and safe-mode suppression. The script with `-Pins` verified configured ordering, drag persistence after restart, deduplication, launching a real shortcut, rendered monochrome icon pixels and a native tooltip. An additional live check found packaged Notepad by the title of a temporary test document. The `-Spotlight` test verified the borderless styles, placement, calculation, Escape and focus-loss dismissal. The `-TopBar` test verified the clock, non-activating styles, fullscreen hiding, Spotlight and Exit. The `-AutoHide` test verified active-display placement, pointer reveal and pointer retraction on one attached monitor. The `-Preview` test verified actual DWM thumbnail pixels, focus preservation and cleanup on pointer departure. Live Spotlight, Dock and Top Bar captures were visually reviewed. Cross-build, high contrast and mixed-DPI testing remain pending.

The media smoke test used an isolated local Helium profile and real Windows media-session events. It verified expansion, title and artist, play, pause, next, previous and fullscreen recovery. The Island capture was visually reviewed.

Checked the declared licences of all locked dependencies, including the added Windows Runtime bindings. Cargo Audit passed with warnings denied across 31 lockfile entries.

## Measurements

Six samples at nominal two-second intervals per scenario. Windows CPU was 74-100% during baseline sampling and 94-100% with the launcher visible while development tools were compiling. The launcher recorded 1.36-1.39 MiB of private working set, 13.13-13.17 MiB of total working set and 1.86-1.91 MiB of private bytes. All six launcher CPU and I/O counter readings were zero at the sampled resolution. These results describe this run. Repeat matched baseline, visible and resident scenarios after background work settles before drawing system-wide conclusions.

The Dock was sampled in two baseline/Dock pairs with six samples per scenario at nominal one-second intervals. The Dock was visible with Spotlight hidden. Across both runs, the process used 1.00-1.02 MiB of private working set, 9.43-9.45 MiB of total working set and 1.45-1.59 MiB of private bytes. Sampled Monolith CPU and I/O counters were zero. Baseline system CPU ranged from 0% to 35%, and Dock system CPU from 0% to 49%. This short measurement does not establish a system-wide CPU impact.

After the borderless Spotlight and pin-reordering changes, two matched idle runs provided twelve samples per scenario. The Dock alone used 1.11-1.13 MiB of private working set, 9.90-9.93 MiB of total working set and 1.60-1.74 MiB of private bytes. Standalone Spotlight used 1.37-1.41 MiB, 13.33-13.36 MiB and 1.88-1.91 MiB respectively. Monolith CPU, GPU and I/O counters read zero at sample resolution. System busiest-engine GPU ranges were 1-6% at baseline, 2-4% with the Dock and 1-3% with Spotlight. System CPU ranges overlapped across the scenarios.

After adding Shell icons, twelve idle Dock samples across two runs measured 2.05-2.11 MiB of private working set, 19.36-19.41 MiB of total working set and 4.37-4.43 MiB of private bytes. The worker had loaded visible tiles and no preview was open. App CPU, GPU and I/O counters read zero at sample resolution. System busiest-engine GPU ranged from 1% to 15% at baseline and 1% to 4% during Dock sampling. The system values are workload observations.

## Git and recovery

The launcher, benchmark, calculator and window-switcher checkpoints are `e86dc9a`, `5f279bb`, `9b9b555` and `a8cef7b`. The project uses `main` and an SSH remote under the verified personal account. Workstation-specific records, configuration and raw benchmark exports stay local.

The binary needs no elevation and makes no persistent changes to Windows. Dismiss standalone Spotlight or use **Exit** on the Dock to stop the process and release native hooks. Pass `--safe-mode` to bypass configuration and start Spotlight alone. Rename the configuration file to recover defaults. Portable removal consists of closing the process and removing the executable, with optional removal of the local configuration directory.

## Shortcut compatibility

The current Windows build returned Win32 error 1409 on a direct Win+Space registration probe and accepted Ctrl+Alt+Space. An experimental low-level capture showed Spotlight but could not reliably give the search field foreground focus. The published runtime uses `RegisterHotKey` and leaves the reserved chord to Windows. A native desktop test with `-Hotkey` verifies the temporary fallback, suppression of key repeats, typing without a click, repeat-to-dismiss and restoration of focus. Live checks passed with the desktop shell, File Explorer, Helium, VS Code and IntelliJ IDEA.

## Next task

Add media progress from the Windows timeline and load artwork on demand. Win+Space remains the target shortcut. Keep the experimental capture out of the public runtime until foreground activation and immediate typing are stable.
