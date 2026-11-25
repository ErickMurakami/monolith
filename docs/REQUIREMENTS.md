# Monolith — Functional Requirements

Read only the component section relevant to the current task, plus dependencies. These are acceptance criteria, not claims of implementation. AGENTS.md governs execution and safety; DESIGN.md governs approved visual behaviour.

### Spotlight

The intended shortcut is Super+Space.

- Override the Windows input-language selector when this feature is enabled and a reliable implementation is available.
- Open immediately on the configured shortcut.
- Give the search field keyboard focus without a mouse click.
- Pressing the shortcut again dismisses Spotlight.
- Escape restores previous focus where supported.
- Prevent duplicate windows and repeated activation.
- Support application search, open windows, calculator and on-demand file search.
- Support installed packaged applications.
- Keep unrelated Windows shortcuts functional.

Ctrl+Alt+Space is a temporary compatibility fallback, not the final target.

Research PowerToys and Windhawk before accepting shortcut limitations.

### Workspaces and Mission Control

Provide spatial, predictable desktop navigation.

- Workspaces have stable identities and ordering.
- Previous and next workspace navigation must be directional.
- Super+1 through Super+9 select configured workspaces.
- Super+Shift+1 through Super+Shift+9 move the active window.
- Super+Tab opens Monolith Mission Control.
- Application switching remains separate from workspace switching.
- Support workspace creation, removal and naming where reliable integration permits.
- Display actual workspace and window state.
- Preserve native window placement and restoration.
- Support multiple monitors where relevant to the founder.

Study Windhawk Virtual Desktop Helper, GlazeWM and Komorebi.

Prefer interoperating with native virtual desktops where technically viable.

Windows Task View remains a recovery option, not the intended primary experience.

### Touchpad Gestures

Investigate macOS-like directional navigation.

Target behaviour:

- Three-finger horizontal swipe: previous or next workspace.
- Three-finger upward swipe: Mission Control.
- Three-finger downward swipe: show desktop.
- Preserve normal two-finger scrolling, zoom and unrelated gestures.

Prefer supported Windows gesture configuration and native device capabilities.

Avoid duplicate gesture execution.

Do not install custom drivers or intercept unrelated keyboard or pointer input.

Validate on the founder's physical touchpad.

### Dock and Taskbar

Develop the Dock into a functional alternative to the default taskbar.

- Display running and pinned applications.
- Group windows reliably.
- Provide live previews on demand.
- Preserve predictable ordering.
- Support minimisation, restoration and activation.
- Support fullscreen suppression.
- Support configurable monitor placement and auto-hide.
- Provide access to Spotlight, Files and Mission Control.
- Coordinate with the workspace manager.

Investigate reversible taskbar visibility control.

Do not permanently disable Explorer.

Restore the original taskbar when Monolith exits, crashes or enters recovery mode.

### Monolith Files

Build a developer-first alternative to Windows Explorer.

Required functionality:

- Responsive virtualised directory navigation.
- Standard file and directory operations.
- Safe copy, move, rename, creation and deletion.
- Conflict handling, cancellation and recovery.
- Directory history and breadcrumbs.
- Hidden file visibility.
- Quick Look.
- Context menus.
- Open terminal here.
- Open project in a configured IDE.
- WSL path support where practical.
- Removable drives and network paths.

Git integration:

- Detect the repository associated with the current directory.
- Display branch and detached HEAD state.
- Display staged, modified, untracked and conflicted files.
- Display ahead and behind counts against locally known upstream state.
- Aggregate relevant repository status into directory indicators.
- Refresh asynchronously when necessary.
- Do not automatically fetch remote repositories.
- Do not modify repository contents without explicit user action.

Prefer native Rust Git libraries or the installed Git executable after measuring correctness, compatibility and overhead.

Do not run Git commands during rendering.

Do not introduce continuous filesystem indexing.

Use bounded, on-demand discovery and optional caches.

### Quick Look

Preserve and extend the existing preview implementation.

- Space opens and closes previews.
- Support common images, PDFs, plain text and source files.
- Provide bounded decoding and cancellation.
- Avoid arbitrary execution of previewed content.
- Release resources immediately after closing.
- Handle invalid and oversized files gracefully.

### Dynamic Island

Provide useful contextual information without interrupting work.

- Real media title and artist.
- Album artwork.
- Playback state.
- Play, pause, previous and next.
- Accurate progress when available.
- Compact collapsed state.
- Functional expanded state.
- Integration with relevant ongoing activities.

Do not simulate playback state or display unsupported controls as functional.

Use event-driven media subscriptions and on-demand artwork.

Refine icons, geometry and animations during the visual milestone.

### Notifications

Provide a coherent Monolith notification presentation and history.

- Investigate supported notification access APIs.
- Respect user permissions.
- Preserve source application identity.
- Support dismissal and available actions.
- Prevent duplicate notification presentation.
- Do not lose original notifications when Monolith fails.
- Integrate with Island and MonoApp where appropriate.

Investigate reliable alternatives to Windows toast presentation.

Do not suppress native notifications before the alternative has been validated.

### Control Center

Provide actual system controls wherever supported.

Prioritise:

- Audio volume and mute.
- Audio output selection.
- Battery information.
- Display brightness where supported.
- Wi-Fi and Bluetooth controls where permitted.
- Media controls.
- Focus and notification settings.

A button that only opens Windows Settings is a fallback.

Clearly distinguish direct controls from settings shortcuts.

### Android Continuity — MonoApp

Implement useful, secure device continuity.

Prioritise:

- Explicit authenticated device pairing.
- Encrypted local transport.
- Android notification forwarding.
- Relevant ongoing activities.
- Device status.
- Reliable disconnect and revocation.
- Minimal permissions and battery overhead.

Do not build a messaging platform.

MonoApp is a companion integration, not an independent social application.

Do not introduce mandatory cloud infrastructure.

Begin this milestone after core desktop interactions are reliable.

### Dedicated application workspaces

For the founder’s preferred workflow, support launching primary applications into dedicated workspaces according to the configured policy. Fullscreen applications should occupy their own spatial workspace, as described in DESIGN.md. Related dialogs and auxiliary windows must not create unnecessary workspaces. Restore the original desktop context when leaving a dedicated workspace.

### Top Bar

Keep clock, date and system indicators at the upper right; the Island remains centred. The floating Dock must not contain the system tray.

## Performance acceptance criteria

Performance is a product feature.

Rust is the implementation language, not evidence of superior performance by itself.

Optimise actual workloads.

- Prefer events over polling.
- Render only when necessary.
- Keep potentially blocking operations off the GUI thread.
- Bound background queues and caches.
- Use lazy initialisation.
- Avoid repeated system enumeration.
- Share existing window inventory and system state.
- Decode media and previews on demand.
- Stop inactive timers.
- Release handles, hooks, subscriptions and native resources.
- Avoid unnecessary memory allocations in hot paths.
- Measure before introducing optimisations.

Initial engineering targets:

- Idle resident memory: preferably below 50 MiB.
- Idle average CPU: preferably below 0.1%.
- Idle GPU usage: as close to zero as practical.
- Warm Spotlight opening: target below 50 ms.
- Smooth interaction animations at the display refresh rate.
- No mandatory continuous filesystem indexing.

These are targets, not existing measurements or reasons to compromise correctness.

Benchmark idle and active workloads separately.

Track:

- Private bytes.
- Private working set.
- Total working set.
- CPU time.
- GPU activity.
- Disk I/O.
- Handles and threads.
- Interaction latency.
- Resource growth across repeated operations.

Use equivalent system baselines.

When comparing with Windhawk, measure the whole system and affected processes, not merely the visible Windhawk executable.

Do not claim performance superiority without comparable evidence.

## Native validation

A feature is not complete merely because it compiles or displays a window.

Validate the intended user interaction.

Use focused tests during development and full validation at milestone boundaries:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

Run relevant native smoke tests.

Validate:

- Real shortcut behaviour.
- Focus restoration.
- Window activation.
- Workspace ordering.
- Fullscreen behaviour.
- DPI scaling.
- Resource cleanup.
- Error handling.
- Explorer compatibility.
- Recovery after failure.
- Repeated interactions.

Test physical hardware when gestures or device behaviour require it.

Do not weaken tests to obtain successful builds.

Record untested scenarios explicitly.

A fallback, placeholder, mock or settings shortcut must not be reported as full implementation of the intended feature.