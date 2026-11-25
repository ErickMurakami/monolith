# Monolith — Accelerated Delivery Sprint

## Objective

Deliver a usable personal daily-driver alpha by 29 November 2026.

Prioritise working desktop interactions, native performance, stability and the founder-approved visual direction.

This sprint does not replace AGENTS.md, REQUIREMENTS.md, REFERENCES.md or DESIGN.md.

It establishes short-term delivery priorities.

## Execution Strategy

- Resume from the actual repository state.
- Reconcile MONOLITH_STATUS.md with existing commits and implementations.
- Preserve validated functionality.
- Research proven upstream implementations before solving complex integrations.
- Adapt relevant Windhawk mechanisms into idiomatic Rust where legally permitted.
- Reuse compatible existing Rust implementations where appropriate.
- Minimise speculative architecture and duplicated research.
- Implement complete vertical slices.
- Continue autonomously after each validated commit.
- Avoid unnecessary branches and worktrees.
- Do not stop after producing plans or reports.

## P0 — Essential Daily-Driver Functionality

These requirements take priority over visual polish and secondary integrations.

- [ ] Reliable Super+Space interception.
- [ ] Immediate Spotlight keyboard focus.
- [ ] Predictable Spotlight dismissal and focus restoration.
- [ ] Spatial workspace management.
- [ ] Dedicated application workspaces according to the approved workspace policy.
- [ ] Keyboard-driven workspace navigation.
- [ ] Three-finger directional touchpad gestures.
- [ ] Reliable Dock application activation.
- [ ] Dock and workspace coordination.
- [ ] Reversible Windows taskbar visibility control.
- [ ] Reliable shutdown and recovery.
- [ ] Stable combined desktop runtime.

## P1 — Developer Productivity

- [ ] Complete Files operations.
- [ ] Validate file conflict handling.
- [ ] Implement on-demand Git repository detection.
- [ ] Display branch and repository status.
- [ ] Display staged, modified, untracked and conflicted files.
- [ ] Open terminal and configured IDE from Files.
- [ ] Preserve existing Quick Look functionality.
- [ ] Maintain responsive directory navigation.

## P2 — Desktop Experience

- [ ] Complete Dynamic Island behaviour.
- [ ] Refine actual media controls.
- [ ] Complete supported Control Center operations.
- [ ] Implement native PC notification presentation.
- [ ] Present incoming notifications below the Island.
- [ ] Implement notification history.
- [ ] Position clock and system indicators in the upper-right status area.

## P3 — Visual Refinement

Follow docs/design/DESIGN.md and the approved reference images.

- [ ] Consistent native typography.
- [ ] Unified vector iconography.
- [ ] Approved Dynamic Island appearance.
- [ ] Approved Spotlight appearance.
- [ ] Approved floating Dock.
- [ ] Approved Files appearance.
- [ ] Approved spatial workspace presentation.
- [ ] Approved notification presentation.
- [ ] Purposeful animations.
- [ ] Consistent geometry and spacing.

Do not introduce permanent rendering loops or unnecessary runtime dependencies.

## P4 — Android Continuity

Begin when the core desktop experience is sufficiently stable.

- [ ] Authenticated MonoApp pairing.
- [ ] Encrypted local communication.
- [ ] Android notification forwarding.
- [ ] Device connection status.
- [ ] Integrated PC and Android notification history.
- [ ] Android notifications displayed below the Dynamic Island.
- [ ] Reliable disconnect and revocation.

Do not fabricate application-specific activity data.

Do not delay the desktop alpha solely because Android integration remains incomplete.

## Research Acceleration

For every complex integration:

1. Consult docs/REFERENCES.md.
2. Search relevant upstream source files.
3. Identify the minimum working mechanism.
4. Verify licence and compatibility requirements.
5. Adapt the mechanism into native Rust.
6. Validate actual Windows behaviour.
7. Record reusable findings.

Avoid repeatedly investigating previously rejected approaches unless new evidence justifies it.

## Blocker Handling

If a task becomes blocked:

- Record the exact technical blocker.
- Record attempted approaches and relevant evidence.
- Identify whether founder approval or physical hardware testing is required.
- Continue independent tasks where possible.
- Return to the blocked task when additional evidence is available.

Never bypass security, compatibility or recovery requirements to meet the deadline.

Do not report temporary fallbacks as completed functionality.

## Token Budget

- Read only the documentation relevant to the active task.
- Use targeted source searches.
- Avoid repeated repository-wide scans.
- Use RTK when available and appropriate.
- Preserve concise technical findings.
- Prefer focused tests during implementation.
- Run full validation at milestone boundaries.
- Use parallel subagents only when their expected benefit justifies their token consumption.

## Release Acceptance

The daily-driver alpha must:

- Launch reliably on the founder's Windows installation.
- Support clean exit and recovery.
- Preserve normal Windows operation when disabled.
- Provide reliable primary desktop interactions.
- Display real runtime information.
- Preserve existing validated functionality.
- Follow the approved visual direction.
- Include a tested release build.
- Document unresolved limitations.
- Include representative performance measurements.

A smaller reliable alpha is preferable to a larger unstable implementation.

## Reporting

After meaningful checkpoints, update MONOLITH_STATUS.md with:

- Completed work.
- Actual validation.
- Remaining blockers.
- Commit reference.
- Exact next task.

Do not duplicate progress reports across multiple documents.

The deadline is a prioritisation target, not permission to compromise engineering quality.