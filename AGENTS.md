# AGENTS.md — Monolith

## 1. Product Mission

Monolith is a native, Rust-first, open-source desktop experience for Windows 11.

Its mission is to combine:

- The modification capabilities of Windhawk.
- The customisation freedom and productivity of Linux and Hyprland.
- The interaction quality of macOS.
- The minimalist industrial design of Nothing OS.
- The compatibility, updates and hardware support of Windows.

Monolith is not merely a collection of floating desktop utilities.

It must progressively replace undesirable Windows interaction behaviours with coherent, efficient native alternatives while preserving essential operating system infrastructure.

Windows with steroids. Linux freedom without Linux maintenance overhead.

## 2. Current Phase — Personal Daily Driver

The immediate objective is to complete and improve the existing Monolith implementation for the project founder.

Prioritise the founder's actual requirements, hardware, Windows version and development workflow.

Do not generalise the product prematurely.

Do not invest in public SDKs, marketplaces, generic plugin engines or hypothetical community requirements.

The repository remains open source and forkable. Community readiness is a later milestone requiring explicit founder approval.

Success means the founder can comfortably use Monolith every day instead of relying on the default Windows desktop experience.

Feature count alone does not indicate completion.

## 3. Accelerated Development Mode

Work autonomously and continuously through the established requirements.

- Resume from MONOLITH_STATUS.md.
- Inspect the current branch and working tree.
- Preserve existing validated functionality.
- Identify incomplete features and actual usability problems.
- Research existing implementations before solving complex Windows integration problems.
- Implement complete vertical slices.
- Use independent subagents when available and justified by the token budget.
- Parallelise independent tasks without conflicting file ownership.
- Validate, commit and continue to the next task.
- Do not stop after each routine implementation.
- Do not request approval for ordinary engineering decisions.
- Do not produce unnecessarily verbose reports.

Continue until the current milestone is completed, the execution budget is exhausted or a genuine blocker requires founder intervention.

Do not confuse rapid implementation with incomplete implementation.

## 4. Reference-Driven Engineering

Do not rediscover solved Windows integration problems.

The following projects are primary technical references:

- Windhawk: https://github.com/ramensoftware/windhawk
- Windhawk Mods: https://github.com/ramensoftware/windhawk-mods
- PowerToys: https://github.com/microsoft/PowerToys
- GlazeWM: https://github.com/glzr-io/glazewm
- Komorebi: https://github.com/LGUG2Z/komorebi
- AutoHotkey: https://github.com/AutoHotkey/AutoHotkey
- Windows samples: https://github.com/microsoft/Windows-classic-samples
- Windows Rust bindings: https://github.com/microsoft/windows-rs

Relevant starting points:

- Windhawk Virtual Desktop Helper.
- Windhawk Keyboard Shortcut Actions.
- Windhawk Taskbar Multi Tray.
- Windhawk Taskbar Tray System Icon Tweaks.
- Windhawk Taskbar Music Lounge.
- Windhawk dedicated-process tool architecture.
- PowerToys Keyboard Manager.
- PowerToys Run and Command Palette.
- GlazeWM and Komorebi window-management implementations.

For complex integrations:

1. Locate an existing working implementation.
2. Identify the relevant Windows APIs, interfaces and event model.
3. Understand compatibility assumptions and limitations.
4. Verify the source licence.
5. Implement an idiomatic Rust equivalent where legally permitted.
6. Integrate it into the existing Monolith architecture.
7. Validate actual behaviour on the founder's Windows build.

Do not mechanically translate entire C++ projects.

Prefer understanding and adapting proven mechanisms.

If source reuse is incompatible with Monolith's licence, use independently implemented alternatives based on permissible documentation.

Record necessary attribution and licence notices.

Monolith must remain independent of Windhawk, PowerToys and other external runtime tools.

## 5. Native Integration and Behaviour Overrides

Treat Windows UX limitations as engineering problems to investigate.

A documented fallback is not necessarily a completed feature.

Use the following integration preference order:

1. Documented native Windows APIs.
2. Existing documented configuration and remapping mechanisms.
3. Reversible user-session integration.
4. Isolated compatibility adapters for unsupported interfaces.
5. Narrow, explicitly approved invasive integration where no safer alternative satisfies the requirement.

Investigating and reviewing undocumented interfaces is permitted.

Experimental implementation and testing must remain isolated and disabled by default until compatibility and recovery have been established.

Do not enable invasive hooks, DLL injection, Explorer patching or persistent system modifications without explicit approval.

Never introduce indiscriminate process injection or a generic hooking engine merely because Windhawk uses one.

When deeper integration is necessary:

- Target only the relevant component.
- Detect Windows build and runtime capabilities.
- Define ownership and resource lifetimes.
- Preserve original behaviour outside the configured override.
- Provide clean shutdown and failure recovery.
- Disable incompatible functionality independently.
- Never require users to block Windows updates.

Keep the original Windows infrastructure available for compatibility and recovery.

## 6. Architecture

Preserve the existing native Rust codebase.

Do not restart the project or rewrite validated components without a measurable engineering reason.

Separate:

- Desktop integration.
- Feature behaviour.
- Rendering.
- Configuration.
- Resource ownership.
- Compatibility and recovery.

Consolidate shared native integration only where multiple components genuinely require it.

Avoid speculative frameworks and excessive crate fragmentation.

Use Win32, WinRT, COM, DWM and other appropriate native APIs.

Prefer safe Rust. Encapsulate necessary unsafe code behind narrow, documented boundaries.

Do not introduce:

- Electron.
- Chromium.
- WebView-based UI.
- Unnecessary runtime dependencies.
- Permanent background rendering loops.
- Unnecessary administrative privileges.
- Kernel drivers.
- Replacement system services.

Preserve Windows kernel, drivers, compositor, security infrastructure, filesystem, networking and Windows Update.

## 7. Functional Requirements and Acceptance Criteria

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

## 8. Performance Requirements

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

## 9. Windows Compatibility and Recovery

Monolith must never become a reason to avoid Windows updates.

- Preserve essential Windows components.
- Keep system overrides reversible.
- Maintain safe mode.
- Support clean application exit.
- Detect unsupported Windows builds.
- Disable failed integrations independently.
- Handle Explorer restart.
- Handle sleep, resume and session changes.
- Preserve access to the native desktop.
- Avoid persistent system changes unless explicitly approved.

Validate restoration after normal exit and unexpected termination.

If an experimental override cannot provide reliable recovery, keep it disabled.

## 10. Development Order

Continue from the existing implementation.

Prioritise:

1. Correct broken and incomplete interactions.
2. Resolve Spotlight shortcut and focus behaviour.
3. Implement meaningful workspace management and gesture integration.
4. Integrate Dock and taskbar behaviour.
5. Complete Files operations and Git awareness.
6. Complete Island and Control Center.
7. Implement integrated notifications.
8. Implement Android Continuity.
9. Validate the complete daily-driver experience.
10. Perform dedicated UI/UX refinement.
11. Measure and optimise.
12. Prepare community distribution after founder approval.

Independent tasks may be developed in parallel.

Do not discard currently active work merely to reorder milestones.

## 11. Visual Identity

Monolith combines extreme minimalism with purposeful industrial design.

It is not a macOS clone.

Established palette:

Dark:
- Background: #080A0D
- Surface: #141820
- Foreground: #E5E7EB

Light:
- Background: #F1F3F6
- Surface: #FFFFFF
- Foreground: #181C22

Use restrained blue accents.

Maintain:

- Consistent monochrome icons.
- Clear typography.
- Compact, predictable geometry.
- Accessible interaction states.
- Purposeful animations.
- Optional original application icons.

Avoid:

- AI slop.
- Generic dashboard aesthetics.
- Decorative cards.
- Excessive transparency.
- Neon colours.
- Coloured borders.
- Decorative gradients.
- Emoji system icons.
- Unnecessary shadows.

Functional completeness comes before extensive visual polish.

Keep rendering separate from behaviour so the visual layer can be refined without rewriting system integration.

Evaluate Direct2D and DirectWrite when measurable rendering limitations justify their introduction.

## 12. Testing and Definition of Done

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

## 13. Token Efficiency

Treat tokens as limited engineering resources.

- Read MONOLITH_STATUS.md first.
- Inspect only relevant source files.
- Use targeted searches.
- Prefer focused diffs.
- Avoid repository-wide repeated scans.
- Filter successful build logs.
- Preserve relevant error details.
- Avoid lengthy planning documents.
- Avoid repeating known context.
- Avoid unnecessary confirmation requests.
- Keep completion summaries concise.
- Use subagents selectively rather than indiscriminately.

Use RTK when installed and appropriate:

```powershell
rtk git status
rtk git diff
rtk cargo check
rtk cargo test
rtk cargo clippy
```

Fall back to normal commands when filtering hides necessary information.

RTK is a development tool, never a Monolith runtime dependency.

## 14. Git Workflow

- Inspect the current branch before modifying files.
- Continue the existing authorised branch.
- Preserve local changes.
- Avoid unnecessary branches and worktrees.
- Use atomic, meaningful commits.
- Use Conventional Commits in English.
- Do not artificially split commits.
- Do not rewrite published history.
- Do not force-push.
- Merge only authorised and validated branches.
- Remove completed branches only after confirming safe integration.

Keep the repository in a resumable state.

## 15. Project Continuity

MONOLITH_STATUS.md is the authoritative implementation checkpoint.

Keep it aligned with actual behaviour and this document.

Record:

- Completed requirements.
- Partially implemented requirements.
- Known limitations.
- Validation results.
- Relevant architectural decisions.
- Benchmark results.
- Exact next task.
- Current commit and branch when useful.

Update docs/architecture.md when architectural decisions change.

Update README.md when user-facing functionality changes.

Do not create redundant progress documents.

Use British English for documentation and source-code terminology.

## 16. Autonomy and Approval

Routine implementation, research, tests, bug fixes, refactoring and established roadmap tasks are authorised.

Request approval only for meaningful changes involving:

- Fundamental architectural replacement.
- Major runtime dependencies.
- Destructive operations.
- Persistent system modifications.
- Invasive process integration.
- Material security or compatibility risks.
- Expansion beyond the agreed product scope.

Investigate alternatives before requesting approval or reporting blockers.

Keep experimental integrations isolated.

## 17. Future Open-Source Community

Anyone may fork and customise Monolith.

Do not build a generic mod marketplace or public plugin SDK during the personal daily-driver phase.

After founder approval, improve:

- General configuration.
- Hardware compatibility.
- Accessibility.
- Localisation.
- Installation and updates.
- Contribution guidelines.
- Public benchmarks.

Upstream contributions must improve general usability, performance, stability, compatibility, security or maintainability.

Personal preferences without general benefit belong in configuration or individual forks.

## 18. Completion Reports

After completing a meaningful checkpoint, report only:

- Implemented functionality.
- Actual validation.
- Remaining limitations.
- Commit reference.
- Next task.

Then continue implementation when authorised and execution budget permits.

The goal is a reliable daily-driver desktop experience, not an impressive development report.
