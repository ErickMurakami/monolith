# Monolith — Technical References

Read only the relevant entry. This is a research index and durable cache, not a claim that upstream implementations have been validated on Monolith.

## Research policy

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

## Fast-track workflow

- Check existing notes below and docs/architecture.md before searching again.
- Search the relevant upstream source using GitHub search, rg in a local checkout or direct file retrieval. Read narrow ranges and follow only necessary call sites.
- Prefer upstream Rust implementations where applicable; otherwise map the required C/C++ APIs, ABI, event model and ownership into windows-rs. Do not mechanically translate infrastructure.
- Record findings once, with a pinned commit and exact path; reuse them until the Windows build or upstream implementation changes materially.
- Keep upstream checkouts outside the Monolith repository; do not add runtime dependencies on reference tools. Never execute unreviewed upstream code.
- Verify licences before copying or adapting source. If incompatible, independently implement from permissible documentation; record attribution as required.
- Stop broad research when a viable approach and a focused validation plan exist. Investigate further only for a concrete blocker.
- Keep invasive integration subject to AGENTS.md approval, isolation and recovery requirements.

## Lookup index (starting points, not verified implementations)

| Component | First references | Investigate |
|---|---|---|
| Spotlight shortcut and focus | [PowerToys](https://github.com/microsoft/PowerToys), [Windhawk Mods](https://github.com/ramensoftware/windhawk-mods) | Keyboard Manager, Keyboard Shortcut Actions; chord suppression, focus, repeat and recovery |
| Spotlight search | PowerToys Run / Command Palette | App discovery, ranking, cancellation |
| Workspaces / Mission Control | [Virtual Desktop Helper](https://github.com/ramensoftware/windhawk-mods/blob/main/mods/virtual-desktop-helper.wh.cpp), [Komorebi](https://github.com/LGUG2Z/komorebi), [GlazeWM](https://github.com/glzr-io/glazewm) | Desktop identity, ordering, switching, window moves, build compatibility |
| Touchpad | Windows Settings and Windows documentation; GlazeWM / Komorebi | Precision touchpad configuration and directional desktop navigation; physical validation |
| Dock / taskbar | Windhawk Taskbar Multi Tray, Taskbar Tray System Icon Tweaks | Visibility, Explorer restart, app grouping, recovery |
| Island | Windhawk Taskbar Music Lounge; Windows media session APIs | Metadata, transport and lifecycle |
| Files / Quick Look | PowerToys Peek, Windows samples | Preview, shell operations, cancellation, large directories |
| Git | [Git documentation](https://git-scm.com/docs/git-status), Rust Git libraries | Porcelain status, branch and upstream state, bounded async queries |
| Control Center | Windows samples, Core Audio, WinRT | Supported direct controls and capability detection |
| Notifications | Windows notification APIs, Windows samples | Permissions, listener lifecycle, native presentation and failure recovery |
| MonoApp | Android NotificationListenerService and Android platform documentation | User consent, lifecycle, pairing and encrypted local transport |
| Native rendering / DPI | [windows-rs](https://github.com/microsoft/windows-rs), [Windows samples](https://github.com/microsoft/Windows-classic-samples) | Win32, DWM, Direct2D, DirectWrite |

## Research records

Add a short entry only after meaningful investigation. Do not duplicate architecture decisions.

```markdown
### <component / problem>
- Upstream: <repository, commit, exact file or official documentation URL>
- Licence / reuse decision: <verified result>
- Mechanism: <APIs, event model, ownership>
- Rust adaptation: <chosen approach and relevant Monolith modules>
- Compatibility: <Windows build, constraints and recovery>
- Evidence: <tested / not tested; exact test or result>
- Next step: <remaining blocker or implementation task>
```

No upstream research record is pre-populated as validated. Existing Monolith test results remain in MONOLITH_STATUS.md.