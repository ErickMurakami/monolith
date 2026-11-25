# Monolith status

## Current milestone

Foundation and the first native application are implemented. Early Desktop Experience work has delivered the experimental Spotlight application launcher, calculator and window switcher.

- `src/windows.rs` owns the Win32 window lifecycle, DPI handling, rendering and shortcut registration.
- `src/search.rs` discovers Start menu desktop shortcuts and ranks local matches.
- `src/calculator.rs` evaluates bounded arithmetic expressions entered in Spotlight.
- `src/desktop.rs` discovers eligible windows and their display, detects fullscreen geometry and switches on explicit selection.
- `src/config.rs` validates TOML, preserves invalid files and supports a safe start.
- `benchmarks/measure.ps1` samples Windows and Monolith memory, CPU and I/O.
- `docs/spotlight.png` shows the running release build with a verification shortcut.

Dock, Top Bar, Island, workspace management, Files, indexing, tuning, settings UI, Android integration and installers are pending.

## Validation

Validated on native Windows 11 26H2. Cargo formatting, Clippy with warnings denied, seven unit tests and the locked release build passed. A live desktop run verified shortcut launch, safe-mode exit and calculator output. `tests/desktop-smoke.ps1` verified filtering out hidden and owned windows, identifying fullscreen and minimised windows, refreshing after a window closes and switching to a minimised window. The launcher display capture was visually reviewed. Cross-build, high contrast and mixed-DPI testing remain pending.

Checked the declared licences of all locked dependencies and queried OSV for all 19 registry package versions. The query returned zero advisory matches.

## Measurements

Six samples at nominal two-second intervals per scenario. Windows CPU was 74-100% during baseline sampling and 94-100% with the launcher visible while development tools were compiling. The launcher recorded 1.36-1.39 MiB of private working set, 13.13-13.17 MiB of total working set and 1.86-1.91 MiB of private bytes. All six launcher CPU and I/O counter readings were zero at the sampled resolution. These results describe this run. Repeat matched baseline, visible and resident scenarios after background work settles before drawing system-wide conclusions.

## Git and recovery

The launcher, benchmark and calculator checkpoints are `e86dc9a`, `5f279bb` and `9b9b555`. The project uses `main` and an SSH remote under the verified personal account. Workstation-specific records, configuration and raw benchmark exports stay local.

The binary needs no elevation and makes no persistent changes to Windows. Close the window to stop the process. Pass `--safe-mode` to skip configuration and disable the global shortcut. Rename the configuration file to recover defaults. Portable removal consists of closing the process and removing the executable, with optional removal of the local configuration directory.

## Next task

Use the desktop inventory in an opt-in Dock. Validate the event-driven refresh, fullscreen suppression and clean exit on the native desktop.
