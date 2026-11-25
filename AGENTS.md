# AGENTS.md — Monolith

## Mission and current phase

Monolith is a Rust-first, open-source Windows 11 desktop experience: Windhawk-like modification capabilities, Linux/Hyprland freedom, macOS interaction quality and Nothing OS minimalism, preserving Windows compatibility, updates and hardware support.

Complete the founder’s personal daily driver first. Preserve the existing implementation; fix actual usability problems rather than maximise feature count. No generic plugin engine, marketplace, public SDK, legacy Windows support or premature community generalisation. The repository remains forkable; community readiness requires explicit founder approval. Upstream contributions must benefit general usability, performance, security, stability, compatibility or maintainability; individual preferences belong in configuration or forks.

## Context routing (mandatory)

- Start with `git status`, current branch and the latest checkpoint / next task in `MONOLITH_STATUS.md`. Read historical measurements only when relevant.
- Read the relevant component heading in `docs/REQUIREMENTS.md` before implementation or changing behaviour. Read associated acceptance criteria and dependencies, not unrelated components.
- For complex Windows integration, consult the relevant entry and existing findings in `docs/REFERENCES.md` before new research.
- Consult `docs/architecture.md` for affected modules and decisions.
- For visual changes, consult the relevant section of `docs/design/DESIGN.md` and only the corresponding approved PNG. Written design corrections override mockups.
- These documents are binding requirements within their scope, not optional suggestions. Do not treat moving details out of this file as removing them. If they conflict with stale status or roadmap notes, follow the current founder-approved requirements and reconcile documentation.
- Do not load all referenced documents or images automatically. Use targeted searches and narrow reads.

## Accelerated execution

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

## Reference-driven development

Research proven upstream implementations before designing complex integrations. Prioritise Windhawk, PowerToys, GlazeWM and Komorebi according to docs/REFERENCES.md. Identify the minimum required native APIs and adapt into idiomatic Rust; verify licences, attribution and compatibility before reuse. Never mechanically translate unrelated C++ infrastructure or introduce upstream tools as runtime dependencies. Save concise findings with exact source paths and commits so subsequent sessions do not repeat research. Use the first viable approach that meets requirements and safety, then validate rather than research indefinitely.

## Native integration and safety

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

## Architecture

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

## Performance and recovery

Performance is a product requirement, not a property guaranteed by Rust. Prefer event-driven, on-demand work; bound queues and caches; share inventory; keep blocking work off the GUI thread; stop inactive timers and release native resources. No continuous filesystem indexing by default. Measure significant changes against equivalent baselines; see performance acceptance criteria in docs/REQUIREMENTS.md.

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

## Development order

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

## Visual direction

The six approved images and `docs/design/DESIGN.md` are the definitive visual target. Preserve dark/light palette, monochrome icons, restrained blue, native rendering and separation of behaviour from UI. No permanent branding, generic web dashboards, neon, decorative gradients or AI slop. Island centred at the top; compact incoming notifications below the player without moving it; clock/date/system indicators upper right; independent floating Dock bottom centre. Fullscreen application workspaces follow spatial navigation. Wallpaper is configurable. Use actual runtime data, never hardcoded mockup content. Functional correctness before polish; temporary native controls are not final design. Consider Direct2D/DirectWrite selectively when justified; no premature renderer migration. See DESIGN.md for all visual requirements and acceptance criteria.

## Validation

Every functional change must meet the applicable acceptance criteria in docs/REQUIREMENTS.md. Run focused tests during development, relevant native smoke tests, and full validation at milestone boundaries:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

Compilation, mocks, settings shortcuts and fallbacks do not establish completion. Record untested scenarios; never weaken tests. Validate actual Windows behaviour and physical touchpad when required. Verify recovery and visual appearance using real Windows screenshots for visual milestones.

## Token efficiency

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

## Git workflow

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

## Continuity

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

## Autonomy and approval

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

## Completion reports

After completing a meaningful checkpoint, report only:

- Implemented functionality.
- Actual validation.
- Remaining limitations.
- Commit reference.
- Next task.

Then continue implementation when authorised and execution budget permits.

The goal is a reliable daily-driver desktop experience, not an impressive development report.

## Active Delivery Sprint

When docs/SPRINT.md exists, use it as the active short-term delivery priority.

Complete the highest-priority unblocked tasks first.

The sprint does not override architecture, security, recovery, licensing, approved design or functional acceptance requirements.

Remove or archive the sprint document after the milestone is completed.