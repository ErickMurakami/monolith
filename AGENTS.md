# AGENTS.md — Monolith

## Project Objective

Monolith is a lightweight, native Windows 11 desktop companion written in Rust.

Its objective is to deliver a cohesive, minimalist desktop experience without replacing Windows itself or introducing unnecessary runtime dependencies.

Prioritise:

- Native performance.
- Low CPU, GPU and memory consumption.
- Reliability and predictable behaviour.
- Modular architecture.
- Windows compatibility.
- Functional completeness before visual refinement.

## Development Workflow

Before implementing a feature:

1. Read `MONOLITH_STATUS.md`.
2. Consult `docs/architecture.md` when architectural context is required.
3. Inspect existing implementations before introducing new abstractions.
4. Research established Windows implementations when dealing with complex system integration.
5. Implement the smallest complete solution.
6. Validate the implementation with relevant tests.
7. Update documentation and project status.
8. Commit the validated changes.

Continue from the current project checkpoint. Do not restart completed milestones or redesign working components without a technical justification.

## Research Before Implementation

Do not reinvent existing solutions unnecessarily.

When implementing complex Windows desktop functionality, consult relevant established open-source projects, including:

- Windhawk: https://github.com/ramensoftware/windhawk
- Windhawk Mods: https://github.com/ramensoftware/windhawk-mods
- PowerToys: https://github.com/microsoft/PowerToys
- GlazeWM: https://github.com/glzr-io/glazewm
- Komorebi: https://github.com/LGUG2Z/komorebi
- Windows implementation examples: https://github.com/microsoft/Windows-classic-samples
- Rust Windows bindings: https://github.com/microsoft/windows-rs

Prioritise research for:

- Global keyboard shortcuts.
- Windows Explorer integration.
- Taskbar visibility and behaviour.
- Virtual desktops and workspaces.
- Window management.
- Media sessions.
- Audio and system controls.
- Multi-monitor support.
- DPI handling.
- Native rendering.
- Focus and foreground activation.

Use these projects as technical references, not mandatory dependencies.

Identify the relevant implementation, understand its limitations and adapt the approach to Monolith's native Rust architecture.

Verify licensing before reusing any source code. Preserve required attribution and licence notices.

Prefer documented Windows APIs. Undocumented APIs, DLL injection, invasive hooks and Explorer patching require explicit technical justification and user approval.

Monolith must remain independent of Windhawk.

## Architecture Rules

- Preserve the existing native Rust architecture.
- Prefer Win32, WinRT, DWM and documented Windows APIs.
- Do not introduce Electron, WebView or Chromium-based runtimes.
- Keep business logic, system integration and rendering separated.
- Reuse shared desktop inventory, configuration and rendering infrastructure.
- Avoid unnecessary dependencies and abstractions.
- Maintain clear module ownership.
- Keep potentially blocking operations off the GUI thread.
- Prefer event-driven updates over continuous polling.
- Bound asynchronous queues, caches and background workloads.
- Release native resources, handles, hooks and subscriptions correctly.
- Preserve safe-mode behaviour and clean application shutdown.
- Do not require administrator privileges unless explicitly approved.

Do not rewrite working components merely to introduce a different architectural pattern.

## Performance

Performance is a core product requirement.

- Avoid permanent rendering loops.
- Render on demand whenever possible.
- Decode artwork, thumbnails and other expensive resources only when required.
- Avoid unnecessary background workers.
- Stop timers and subscriptions when their associated feature is inactive.
- Preserve lazy initialisation for optional components.
- Benchmark significant architectural or runtime changes.
- Compare equivalent baseline and application workloads.
- Never present sampled zero utilisation as proof of zero resource consumption.
- Do not sacrifice responsiveness to reduce memory usage artificially.

Visual improvements must not introduce unjustified runtime overhead.

## Token Efficiency

Treat context and token consumption as limited engineering resources.

- Read only files relevant to the current task.
- Do not repeatedly scan the entire repository.
- Prefer targeted searches using `rg` and `git grep`.
- Inspect focused diffs instead of repeatedly reading complete files.
- Avoid dumping large logs into the conversation.
- Filter successful build output and retain relevant errors.
- Run focused tests during implementation.
- Run broader validation at milestone boundaries.
- Avoid repeating architectural explanations already documented.
- Keep progress reports concise.
- Do not generate unnecessary documentation or duplicate existing information.

When RTK is installed and compatible with the current Windows environment, use it to compact command output.

Examples:

```powershell
rtk git status
rtk git diff
rtk cargo check
rtk cargo test
rtk cargo clippy
```

If RTK is unavailable or its filtering hides relevant information, use native commands.

Do not introduce RTK as a Monolith runtime dependency.

## Development Priorities

Follow the existing roadmap and project status.

The general development order is:

1. Functional implementation.
2. Component integration.
3. Stability and compatibility.
4. Visual consistency.
5. UI/UX refinement.
6. Performance validation.
7. Packaging and distribution.

Complete functional milestones before investing significant effort in cosmetic improvements.

Do not delay a working feature because its temporary icons, spacing or animations are not visually perfect.

However, preserve the separation between functionality and rendering so visual refinement does not require rewriting the underlying feature.

## Visual Identity

Monolith has its own identity.

Its design combines extreme minimalism, the functional organisation of macOS and the restrained industrial aesthetics of Nothing OS.

It must not become a macOS clone.

Use the established palette:

Dark theme:

- Background: `#080A0D`
- Surface: `#141820`
- Foreground: `#E5E7EB`

Light theme:

- Background: `#F1F3F6`
- Surface: `#FFFFFF`
- Foreground: `#181C22`

Use blue sparingly for active states, selection and relevant feedback.

Maintain:

- Consistent monochrome system icons.
- Optional original application icons.
- Clear typography.
- Predictable spacing.
- Compact interfaces.
- Accessible interaction states.
- Consistent geometry.
- Subtle, purposeful animations.

Avoid:

- AI slop.
- Excessive cards.
- Decorative gradients.
- Neon colours.
- Coloured borders.
- Excessive transparency.
- Unnecessary shadows.
- Oversized controls.
- Emoji-based system controls.
- Generic web dashboard aesthetics.
- Decorative elements without functional purpose.

Consider Direct2D and DirectWrite selectively during the visual refinement milestone if existing GDI rendering becomes a measurable limitation.

Do not migrate the rendering architecture prematurely.

## Component Responsibilities

Preserve independent and clearly defined responsibilities for:

- Spotlight.
- Dock.
- Dynamic Island.
- Top Bar.
- Control Center.
- Mission Control.
- Workspaces.
- Monolith Files.
- Quick Look.
- File indexing.
- Settings.
- Android Continuity.

Reuse existing infrastructure instead of implementing separate system integrations for each component.

Do not expose unsupported functionality as working functionality.

Where Windows imposes limitations, provide a documented fallback rather than an unreliable implementation.

## Testing and Validation

Every functional change must include appropriate validation.

Use:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```

Run relevant native smoke tests from the `tests/` directory.

Validate actual Windows behaviour whenever a feature depends on:

- Native window activation.
- Global shortcuts.
- System media sessions.
- Audio controls.
- Explorer integration.
- Window thumbnails.
- File operations.
- Multiple displays.
- DPI scaling.

Do not claim a feature is validated if it has only compiled successfully.

Record untested scenarios explicitly.

Do not disable or weaken tests merely to obtain a successful build.

## Git Workflow

- Continue on the existing authorised development branch.
- Do not create new branches or worktrees unnecessarily.
- Inspect the current branch and working tree before making changes.
- Preserve existing user changes.
- Never discard uncommitted work without explicit approval.
- Keep commits atomic and meaningful.
- Use Conventional Commits.
- Write commit messages in English.
- Group related changes into coherent commits.
- Avoid artificial commit splitting.
- Do not force-push or rewrite published history.
- Merge completed development branches into `main` only when authorised.
- Remove completed branches after their changes have been safely merged.

## Project Continuity

`MONOLITH_STATUS.md` is the primary development checkpoint.

Keep it updated with:

- Completed functionality.
- Current milestone.
- Relevant architectural decisions.
- Validation results.
- Known limitations.
- Pending work.
- Latest benchmark results.
- Exact next implementation task.

Before concluding a development session, ensure the repository contains sufficient information for another agent to resume without reconstructing the entire project context.

Do not create redundant progress files.

## Documentation

Write source code, documentation and commit messages in English, preferably British English.

Keep `README.md` aligned with actual implemented functionality.

Do not document planned functionality as already available.

Keep architectural decisions in `docs/architecture.md`.

Avoid verbose documentation that merely restates the source code.

## Agent Autonomy

Proceed autonomously with routine implementation, testing, bug fixes and documented roadmap tasks.

Request explicit approval before:

- Changing the fundamental architecture.
- Introducing a major runtime dependency.
- Performing destructive operations.
- Using undocumented system integration with significant stability risks.
- Introducing persistent modifications to Windows.
- Expanding the project beyond the established roadmap.

When encountering a blocker, investigate established implementations before abandoning the feature.

Prefer a reliable, limited implementation over an ambitious but unstable solution.

## Completion Criteria

A task is complete when:

- The requested functionality works.
- Relevant tests pass.
- Native behaviour has been validated where applicable.
- Resource ownership is correct.
- Existing functionality remains intact.
- Documentation reflects the implementation.
- Known limitations are recorded.
- Changes are committed appropriately.

At the end of each task, report only:

- What was implemented.
- Validation results.
- Remaining limitations.
- Commit reference.
- Next task.

Avoid unnecessarily long completion reports.