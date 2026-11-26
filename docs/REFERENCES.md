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
| Top Bar | Windows Application Desktop Toolbars, Windhawk Custom Top Bar | Native work-area reservation and Shell restart |
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

### Top Bar and desktop work area

- Use Explorer's documented `ABM_NEW`, `ABM_QUERYPOS`, `ABM_SETPOS` and `ABM_REMOVE` messages for a native work-area reservation. The MIT-licensed Windhawk Custom Top Bar uses the same APIs. Monolith implements registration and lifecycle independently from the Windows documentation. [Microsoft Application Desktop Toolbars](https://learn.microsoft.com/en-us/windows/win32/shell/application-desktop-toolbars), [Windhawk Custom Top Bar at f8c7ee7](https://github.com/mrallayi/windhawk-mods/blob/f8c7ee7e56a4018cf528449c88cfa18f4342475f/mods/custom-top-bar.wh.cpp)
- On build 26300.9974, the `TaskbarCreated` broadcast alone did not recover the reservation after terminating and relaunching Explorer. A scoped WinEvent check of the Shell window identity recovered registration on the same build. [Microsoft taskbar creation notification](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar#taskbar-creation-notification)
- IntelliJ IDEA 2026.3 can maximise its native window to the physical top edge while Explorer reports the reserved work area. Win32 permits applications to override maximised geometry, and the JetBrains Runtime implements custom maximised bounds in `AwtFrame::WmGetMinMaxInfo`. Monolith reads the foreground window's DWM frame on WinEvents and yields the strip when it overlaps. The JetBrains source is GPL-2.0 with the Classpath exception and was reviewed by reference only. [Windows window features](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features), [JetBrains Runtime awt_Frame.cpp at 6947004](https://github.com/JetBrains/JetBrainsRuntime/blob/694700444776d93d8fc9436fa4f1d24b951f6518/src/java.desktop/windows/native/libawt/windows/awt_Frame.cpp)

### Dock and native taskbar visibility

- Windhawk Nimbus Menu combines tray visibility with `ABM_SETSTATE` and saves the previous auto-hide preference. Taskbar Multi Tray operates inside Explorer and ShellHost. These sources were reviewed by reference only. [Nimbus Menu at 9e30cde](https://github.com/ramensoftware/windhawk-mods/blob/9e30cde12c8838f10f8bca0cf40b5b213ec1ed54/mods/nimbus-menu.wh.cpp#L1334-L1390), [Taskbar Multi Tray at 9e30cde](https://github.com/ramensoftware/windhawk-mods/blob/9e30cde12c8838f10f8bca0cf40b5b213ec1ed54/mods/taskbar-multi-tray.wh.cpp)
- On build 26300.9974, changing only the verified Explorer tray window's visibility preserved the exact primary work area, foreground window and auto-hide preference. At 120 DPI, the native bottom band was 60 physical pixels. Monolith uses this existing band and an independent recovery process with native wait handles. [Windows taskbar and work area](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar), [ShowWindowAsync](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync), [Session change notification](https://learn.microsoft.com/en-us/windows/win32/termserv/wm-wtssession-change)

### Files copy and move

- Windows defines the `CF_HDROP` file-list format and `CFSTR_PREFERREDDROPEFFECT` for copy and cut. Monolith independently validates the clipboard payload and executes explicit paste actions through `IFileOperation`. `FOF_RENAMEONCOLLISION` and `FOFX_PRESERVEFILEEXTENSIONS` preserve existing targets. `FOFX_ADDUNDORECORD` requests a native undo record for the user's operation. Progress callbacks and the Shell abort flag expose partial operations. [Shell clipboard formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard), [IFileOperation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation), [Operation flags](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags)

### Spotlight shortcut and foreground activation

- The [Windhawk Keyboard Shortcut Actions mod](https://github.com/ramensoftware/windhawk-mods/blob/8265d3ad06d1ad81e736feadea2264f6b3fa4329/mods/keyboard-shortcut-actions.wh.cpp) uses native `RegisterHotKey` inside its taskbar host. The mod was reviewed by reference only.
- [PowerToys Keyboard Manager](https://github.com/microsoft/PowerToys/blob/e3786cb5e287d90fc7c3685cc4887fea2c7d9a08/src/modules/keyboardmanager/KeyboardManagerEngineLibrary/KeyboardEventHandlers.cpp) uses a marker key to prevent a lone logo-key action while remapping. The project uses the MIT licence. Monolith implements its own narrower callback and retains [Windows foreground policy](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow).
- On Windows 11 26H2 build 26300.9974, a direct Win+Space registration and a Win+F24 bridge returned error 1409. Opening through a posted window message and simulating Alt did not transfer foreground. A bridge through the registered Ctrl+Alt+Space chord delivered a native hotkey message that could activate the overlay.
- In isolated tests the bridge activated Spotlight and restored the original foreground. Combined Dock, Top Bar and Island trials lost the first character when typing began 15 ms after the shortcut. Sending the fallback chord from the logo-release callback failed to activate Spotlight. Keep the bridge and its tests isolated until combined first-key delivery is reliable. Physical keyboard and elevated-foreground tests also remain open.

### Directional workspace navigation

- Windhawk's [Virtual Desktop Helper](https://github.com/ramensoftware/windhawk-mods/blob/9e30cde12c8838f10f8bca0cf40b5b213ec1ed54/mods/virtual-desktop-helper.wh.cpp) selects private COM vtables by Windows build. The mod was reviewed by reference only.
- [GlazeWM's workspace command](https://github.com/glzr-io/glazewm/blob/5709ad0a3c7c386bbc3e38166a865ffc12937515/packages/wm/src/commands/workspace/focus_workspace.rs) and [Komorebi's monitor workspace model](https://github.com/LGUG2Z/komorebi/blob/72aa4c97ade72b802a008bdba4bf59cca04df665/komorebi/src/monitor.rs) manage application-owned workspace state. No upstream source was reused.
- Mission Control issues the [documented native directional shortcuts](https://support.microsoft.com/en-us/windows/experience/how-to-multitask-in-windows) on explicit selection. The 26H2 test checked desktop membership through the public COM interface, verified the boundary and restored the original desktop.
- The documented [precision touchpad controller](https://learn.microsoft.com/en-us/windows/win32/input-precisiontouchpad/touchpadgesturescontroller) delivers input to the foreground process. A read-only device probe found an active five-contact touchpad. Global three-finger actions still require a supported mapping and physical validation.
