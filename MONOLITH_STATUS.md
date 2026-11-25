# Monolith status

## Current milestone

Foundation and the first native application are implemented. Desktop Experience work has delivered the experimental Spotlight application launcher, calculator, window switcher and opt-in Dock.

- `src/windows.rs` owns the borderless Spotlight window lifecycle, keyboard navigation, focus dismissal and shortcut registration.
- `src/overlay.rs` shares the native overlay styles, palette and DPI scaling between Spotlight and Dock.
- `src/search.rs` discovers Start menu desktop shortcuts and ranks local matches.
- `src/calculator.rs` evaluates bounded arithmetic expressions entered in Spotlight.
- `src/desktop.rs` discovers eligible windows and their display, detects fullscreen geometry and switches on explicit selection.
- `src/dock.rs` presents ordered, configurable Start menu pins and open windows on the primary display. The Dock handles live updates, minimisation, restoration and fullscreen suppression.
- `src/config.rs` validates TOML, preserves invalid files and supports a safe start.
- `benchmarks/measure.ps1` samples Windows and Monolith memory, CPU and I/O.
- The benchmark also records the busiest GPU engine from WDDM counters when available.
- `docs/spotlight.png` shows a calculation in the running borderless Spotlight release build.
- `docs/dock.png` is a cropped capture of the running Dock with a verification window selected.

The Dock groups running windows by packaged application ID or verified process image and opens a native window chooser. Dock icons, previews, pointer auto-hide and per-display placement are pending. Top Bar, Island, workspace management, Files, indexing, tuning, settings UI, Android integration and installers are pending.

## Validation

Validated on native Windows 11 26H2. Cargo formatting, Clippy with warnings denied, twelve unit tests and the locked release build passed. A live desktop run verified shortcut launch, safe-mode exit and calculator output. `tests/desktop-smoke.ps1` verified filtering out hidden and owned windows, identifying fullscreen and minimised windows, refreshing after a window closes and switching to a minimised window. The same script with `-Dock` verified mouse clicks, minimisation, restoration, title changes, grouped menu selection, fullscreen suppression and recovery, Spotlight opening, clean exit and safe-mode suppression. The script with `-Pins` verified configured ordering, drag persistence after restart, deduplication and launching a real shortcut. An additional live check found packaged Notepad by the title of a temporary test document. The `-Spotlight` test verified the borderless styles, placement, calculation, Escape and focus-loss dismissal. The launcher capture was visually reviewed. Cross-build, high contrast and mixed-DPI testing remain pending.

Checked the declared licences of all locked dependencies. OSV returned zero advisory matches for 19 registry package versions. Cargo Audit also passed against the RustSec database with warnings denied.

## Measurements

Six samples at nominal two-second intervals per scenario. Windows CPU was 74-100% during baseline sampling and 94-100% with the launcher visible while development tools were compiling. The launcher recorded 1.36-1.39 MiB of private working set, 13.13-13.17 MiB of total working set and 1.86-1.91 MiB of private bytes. All six launcher CPU and I/O counter readings were zero at the sampled resolution. These results describe this run. Repeat matched baseline, visible and resident scenarios after background work settles before drawing system-wide conclusions.

The Dock was sampled in two baseline/Dock pairs with six samples per scenario at nominal one-second intervals. The Dock was visible with Spotlight hidden. Across both runs, the process used 1.00-1.02 MiB of private working set, 9.43-9.45 MiB of total working set and 1.45-1.59 MiB of private bytes. Sampled Monolith CPU and I/O counters were zero. Baseline system CPU ranged from 0% to 35%, and Dock system CPU from 0% to 49%. This short measurement does not establish a system-wide CPU impact.

After the borderless Spotlight and pin-reordering changes, two matched idle runs provided twelve samples per scenario. The Dock alone used 1.11-1.13 MiB of private working set, 9.90-9.93 MiB of total working set and 1.60-1.74 MiB of private bytes. Standalone Spotlight used 1.37-1.41 MiB, 13.33-13.36 MiB and 1.88-1.91 MiB respectively. Monolith CPU, GPU and I/O counters read zero at sample resolution. System busiest-engine GPU ranges were 1-6% at baseline, 2-4% with the Dock and 1-3% with Spotlight. System CPU ranges overlapped across the scenarios.

## Git and recovery

The launcher, benchmark, calculator and window-switcher checkpoints are `e86dc9a`, `5f279bb`, `9b9b555` and `a8cef7b`. The project uses `main` and an SSH remote under the verified personal account. Workstation-specific records, configuration and raw benchmark exports stay local.

The binary needs no elevation and makes no persistent changes to Windows. Dismiss standalone Spotlight or use **Exit** on the Dock to stop the process and release native hooks. Pass `--safe-mode` to bypass configuration and start Spotlight alone. Rename the configuration file to recover defaults. Portable removal consists of closing the process and removing the executable, with optional removal of the local configuration directory.

## Next task

Use the shared borderless overlay design for the Top Bar. Extend the Dock with native monochrome icons and per-display placement.
