# Monolith — Visual Design Specification

## 1. Purpose

This document defines the approved visual identity, interaction principles and target user experience of Monolith.

Monolith is a native Rust desktop experience that combines the customisation freedom of Linux, the productivity of Hyprland, the interaction quality of macOS and the minimalist industrial design of Nothing OS, whilst preserving Windows compatibility and reliability.

The objective is not to reproduce macOS or create another Windows theme.

The objective is to establish a coherent, independent desktop experience.

**Windows compatibility. Linux freedom. Native performance.**

The reference images in this directory represent the founder-approved target design, not the current implementation.

## 2. Approved Design References

| Reference | Component |
|---|---|
| `dynamic-island.png` | Dynamic Island compact and expanded states |
| `spotlight.png` | Spotlight search and application launcher |
| `dock.png` | Floating application Dock |
| `files.png` | Developer-first file manager |
| `workspaces.png` | Spatial workspaces and Mission Control |
| `notifications.png` | Desktop and Android notification integration |

These images are the authoritative visual references.

Written requirements in this document take precedence over inconsistencies in the reference images.

Do not introduce alternative visual directions without founder approval.

Do not use reference images as runtime interface assets.

## 3. Design Principles

### Extreme Minimalism

Every visible element must serve a purpose.

Prefer:

- Clean surfaces.
- Consistent geometry.
- Compact controls.
- Clear information hierarchy.
- Generous but efficient spacing.
- Monochrome system iconography.
- Restrained visual feedback.
- Predictable interactions.

Avoid:

- AI-generated dashboard aesthetics.
- Excessive cards and nested containers.
- Decorative gradients.
- Neon effects.
- Excessive glow.
- Unnecessary shadows.
- Coloured decorative borders.
- Oversized buttons.
- Excessive transparency.
- Decorative animations.
- Emoji-based system controls.
- Permanent promotional text or branding.

The interface should feel like a native desktop environment, not a website rendered on the desktop.

### Functional Beauty

Visual quality must improve usability.

Animations, colours, icons and spacing must communicate state, hierarchy or available interactions.

Do not sacrifice responsiveness, resource efficiency or reliability for decorative effects.

### Desktop Independence

The interface must work with different wallpapers, applications and screen resolutions.

The wallpapers in the reference images are illustrative.

Do not bake wallpaper content into interface surfaces.

Do not display a permanent Monolith logo, product name or slogan on the desktop.

## 4. Design System

### Dark Theme

| Token | Colour |
|---|---|
| Background | `#080A0D` |
| Surface | `#141820` |
| Elevated surface | `#1C2129` |
| Primary text | `#E5E7EB` |
| Secondary text | `#9CA3AF` |
| Muted text | `#6B7280` |
| Subtle border | `#292E36` |
| Accent | `#3B82F6` |
| Success | `#22C55E` |
| Warning | `#EAB308` |
| Error | `#EF4444` |

### Light Theme

| Token | Colour |
|---|---|
| Background | `#F1F3F6` |
| Surface | `#FFFFFF` |
| Elevated surface | `#E8ECF1` |
| Primary text | `#181C22` |
| Secondary text | `#59616D` |
| Muted text | `#7B8490` |
| Subtle border | `#DCE1E7` |
| Accent | `#2563EB` |

Dark mode is the primary visual reference during the founder's daily-driver development phase.

Maintain functional light-mode support and refine its visual consistency during the dedicated visual milestone.

### Colour Usage

Blue is reserved for:

- Selected items.
- Active controls.
- Focus indicators.
- Playback progress.
- Active workspaces.
- Relevant interaction feedback.

Semantic colours may be used for Git status, warnings, errors and successful operations.

Do not apply blue to every icon, border or interactive element.

### Typography

Prefer native, legible typography.

Use Segoe UI or an appropriate locally available equivalent.

A restrained monospace typeface may be used for:

- Git branches.
- File paths.
- Keyboard shortcuts.
- Technical metadata.
- Selected compact system information.

Avoid bundling large font packages without justification.

Maintain consistent font weights, line heights and information hierarchy.

### Spacing

Use a consistent spacing scale based on logical pixels:

- 4: compact internal spacing.
- 8: related elements.
- 12: standard internal spacing.
- 16: component padding.
- 24: major content separation.
- 32: large surface separation.

Adapt dimensions to DPI and available screen space.

These values are initial design guidelines, not rigid physical pixel requirements.

### Geometry

Use consistent corner radii:

- Small controls: 6–8 logical pixels.
- Standard controls: 8–12 logical pixels.
- Floating panels: 16–24 logical pixels.
- Compact capsule surfaces: fully rounded.

The Dynamic Island and Dock use capsule-inspired geometry.

Do not apply rounded containers to every individual element.

### Iconography

Use consistent vector-based system icons wherever practical.

Maintain:

- Uniform stroke widths.
- Consistent proportions.
- Clear silhouettes.
- Predictable alignment.
- Appropriate contrast.
- Accessible labels and tooltips.

Prefer monochrome system icons.

Original application icons may retain their colours when enabled or when colour is necessary for application recognition.

Notifications and album artwork may retain their original colours.

Do not use Unicode symbols or emoji as substitutes for properly rendered system icons.

Consider Lucide, Fluent System Icons or independently implemented vector paths, subject to licensing and native rendering requirements.

## 5. Desktop Layout

The desktop consists of three primary positioning zones.

### Upper Centre

Reserved for the Dynamic Island and its temporary notification overlays.

The Island must remain horizontally centred relative to its active display.

### Upper Right

Reserved for:

- Clock.
- Date.
- Battery.
- Connectivity.
- Audio status.
- Notification access.
- Control Center access.
- Paired-device status when appropriate.

Clock and system indicators must never be placed in the bottom Dock.

This requirement overrides the incorrect bottom-right status area shown in some notification references.

### Lower Centre

Reserved for the independently floating Dock.

The Dock must not be embedded in a full-width footer or combined with the original Windows system tray.

### Desktop Content

The remaining desktop area belongs to applications, workspaces and the user's wallpaper.

Do not introduce permanent dashboards, status cards or promotional elements.

## 6. Dynamic Island

Reference: `dynamic-island.png`

The Dynamic Island is a persistent contextual activity surface, not a general-purpose widget container.

It must remain horizontally centred at the top of the display.

### Compact State

Display:

- Small album artwork.
- Track title.
- Artist.
- Playback state.
- Compact playback controls where space permits.
- Discrete playback activity indicator.

The compact state must occupy minimal screen space.

Use a near-black capsule with restrained separation from the surrounding desktop.

Do not use permanent neon borders or glowing outlines.

### Expanded State

Expand naturally from the compact Island.

Display:

- Larger album artwork.
- Track title.
- Artist.
- Album name when available.
- Playback progress.
- Elapsed time.
- Total duration.
- Previous track.
- Play or pause.
- Next track.
- Shuffle and repeat when supported.

The expanded surface should follow the proportions and visual hierarchy of the approved reference.

Only expose controls that the current media session supports.

Allow seeking when supported by the underlying session.

### Interaction

- Clicking the compact Island expands or collapses it.
- Expansion must not unnecessarily interrupt the active application.
- Playback controls must operate on the actual media session.
- Unsupported controls must be hidden or clearly disabled.
- Changing tracks must update artwork and metadata correctly.
- Long titles must truncate gracefully.
- Fullscreen behaviour must respect application compatibility.

### Animation

Expansion and collapse should be smooth and visually continuous.

Prefer short transitions of approximately 150–250 ms.

Do not maintain a rendering loop when the Island is stationary.

An audio activity indicator must not be presented as an actual waveform unless it is based on genuine audio measurements.

### Inactive State

When no relevant media session or activity exists, the Island may collapse or hide according to its configuration.

Do not display fabricated playback information.

## 7. Spotlight

Reference: `spotlight.png`

Spotlight is the primary keyboard-driven application launcher and contextual search surface.

### Layout

Use a centred floating panel with:

- Prominent search input.
- Search icon.
- Active keyboard focus indicator.
- Optional shortcut hint.
- Search categories.
- Grouped results.
- Compact keyboard navigation guidance.

Supported categories should include:

- All.
- Applications.
- Files.
- Open windows.
- Commands or registered actions.
- Web search when explicitly supported.

The calculator may be integrated directly into the search field.

Do not display categories that have no working implementation.

### Search Results

Each result may include:

- Relevant icon.
- Primary name.
- Secondary description or path.
- Contextual action.
- Keyboard activation hint.

The selected result uses a restrained elevated or blue-tinted surface.

Avoid individual decorative cards around every result.

### Interaction

The intended shortcut is `Super+Space`.

- Open immediately.
- Focus the search field automatically.
- Accept keyboard input without requiring a mouse click.
- Navigate using arrow keys.
- Activate using Enter.
- Close using Escape.
- Toggle visibility using the opening shortcut.
- Restore previous focus when appropriate.

The Windows keyboard-language selector must not appear when the Monolith shortcut override is active and supported.

The fallback shortcut is a compatibility measure, not the final interaction target.

### Search Behaviour

Use responsive, bounded, on-demand discovery.

Do not require permanent filesystem indexing.

Web searches must require explicit user interaction.

Commands must not execute automatically as a consequence of typing.

## 8. Dock

Reference: `dock.png`

The Dock is a compact, floating replacement for routine Windows taskbar interactions.

### Layout

- Horizontally centred at the bottom.
- Independent from the upper status area.
- Near-black capsule or rounded surface.
- Consistent icon sizes.
- Predictable icon spacing.
- Subtle separation between application groups and system actions.

Do not create a full-width background behind the Dock.

Do not display the clock, date, battery or system tray inside it.

### Application States

- Pinned applications retain predictable positions.
- Running applications display a small activity indicator.
- The active application receives subtle emphasis.
- Multiple windows are grouped appropriately.
- Original application colours are configurable.

Use small blue indicators for running or selected applications where appropriate.

Avoid oversized highlights and permanent glow effects.

### Interaction

- Click to launch or activate.
- Click an active application to minimise when appropriate.
- Hover to display a preview on demand.
- Select a window from grouped applications.
- Drag to reorder pinned items.
- Open contextual actions using the secondary mouse button.
- Support configurable auto-hide.

Hover magnification, if implemented, must be restrained and must not disrupt adjacent targets.

### Integration

The Dock must coordinate with the workspace manager.

Selecting an application located in another workspace should navigate to the appropriate workspace and activate that application.

Native taskbar visibility overrides must remain reversible.

## 9. Monolith Files

Reference: `files.png`

Monolith Files is a developer-first alternative to Windows Explorer, influenced by the functional organisation of Finder.

Its purpose is to make filesystem navigation and development workflows more efficient.

### Window Structure

Provide:

- Compact native window controls.
- Navigation sidebar.
- Breadcrumb-based location bar.
- Search field.
- File operation toolbar.
- Main content area.
- Contextual repository information.
- Compact status area where useful.

### Sidebar

Include relevant locations:

- Home.
- Desktop.
- Documents.
- Downloads.
- Development.
- Pictures.
- Music.
- Videos.
- Local drives.
- Network locations.
- WSL locations when supported.
- User-defined favourites.

Avoid unnecessary duplicate entries.

### File Toolbar

Provide accessible actions for:

- Create.
- Cut.
- Copy.
- Paste.
- Delete.
- Rename.
- Open terminal.
- Open in configured IDE.
- Additional contextual operations.

Use consistent vector icons and tooltips.

### Directory Content

Prioritise a virtualised list with columns for:

- Name.
- Git status when applicable.
- Size.
- Modification date.

Support sorting and relevant selection states.

Alternative grid and compact views may be introduced where useful.

### Git Awareness

When navigating inside a Git repository, display a compact repository information surface.

Include:

- Repository name.
- Current branch.
- Detached HEAD state when applicable.
- Ahead and behind counts against locally known upstream state.
- Modified files.
- Staged changes.
- Untracked files.
- Conflicts.
- Explicit refresh action.

Individual files and directories may display contextual Git status indicators.

Directory indicators should represent relevant changes within their contents.

Use semantic colours consistently.

Do not display a Git information surface outside a repository.

Do not automatically fetch from remote repositories.

Do not execute Git operations during rendering.

Any operation that changes repository contents or branch state requires explicit user action.

### Quick Look

Pressing Space on a selected file opens its preview.

Support appropriate preview formats, including:

- Images.
- PDFs.
- Text.
- Source code.
- Basic file and directory metadata.

The preview should appear as a clean, floating surface.

Pressing Space or Escape dismisses it.

Preview rendering must be bounded, cancellable and performed on demand.

Do not execute previewed scripts or active document content.

### Performance

Directory navigation must remain responsive in large repositories.

Do not introduce continuous filesystem indexing.

Use shallow directory enumeration, virtualised views, bounded background operations and asynchronous Git queries.

## 10. Workspaces and Mission Control

Reference: `workspaces.png`

Workspaces must provide spatial organisation rather than application switching based on recent usage.

### Workspace Model

Support two conceptual workspace types:

1. Standard desktops containing multiple windows.
2. Dedicated application workspaces containing an application in fullscreen or maximised-workspace mode.

A standard desktop may contain several applications.

Applications opened in dedicated fullscreen mode should occupy their own workspace, following the intended macOS-inspired interaction.

Opening an application must follow the configured workspace policy.

For the founder's preferred workflow, investigate automatic dedicated workspace creation for newly launched primary applications.

Related dialogs, auxiliary windows and secondary application windows must not create unnecessary workspaces.

### Spatial Ordering

Workspaces have stable identities and predictable horizontal ordering.

The previous workspace remains to the left.

The next workspace remains to the right.

Application usage frequency must not silently reorder workspaces.

### Mission Control Layout

Display:

- Ordered workspace navigation.
- Workspace names or numbers.
- Active workspace indication.
- Actual window previews.
- Dedicated fullscreen application previews.
- Workspace creation control where supported.

Standard desktops may display grouped window previews.

Dedicated application workspaces should display their primary application prominently.

Do not display every running application in one undifferentiated grid.

### Interaction

Target keyboard interactions:

| Shortcut | Action |
|---|---|
| `Super+1–9` | Select workspace |
| `Super+Shift+1–9` | Move active window |
| `Super+Left` | Previous workspace |
| `Super+Right` | Next workspace |
| `Super+Tab` | Mission Control |
| `Alt+Tab` | Application switching |

These shortcuts must be configurable.

### Touchpad Gestures

Target interactions:

- Three-finger horizontal swipe: switch spatially between workspaces.
- Three-finger upward swipe: Mission Control.
- Three-finger downward swipe: show desktop.

Preserve normal scrolling, zooming and unrelated gestures.

Avoid duplicate execution by Windows and Monolith.

Gesture behaviour must be validated on actual hardware.

### Workspace Recovery

Closing or leaving a dedicated fullscreen application workspace should restore the appropriate desktop context.

Do not lose windows or alter unrelated application placement.

Preserve access to the original Windows desktop management tools as a recovery mechanism.

## 11. Notifications and Android Continuity

Reference: `notifications.png`

Notifications must provide a consistent presentation regardless of whether they originate from Windows or a paired Android device.

### Notification Sources

Support:

- Native Windows applications.
- Monolith components.
- Development tools.
- Paired Android devices through MonoApp.

Preserve the original source application identity.

Clearly distinguish PC and Android origins.

### Incoming Notification Overlay

Incoming notifications must appear directly below the Dynamic Island.

The media player must remain above the notification.

The notification must not:

- Replace the player unnecessarily.
- Move the player vertically.
- Interrupt playback.
- Steal keyboard focus.
- Occupy excessive desktop space.

Use a compact, near-black rounded surface aligned horizontally with the Island.

The notification should normally be equal to or narrower than the active Island, expanding only when genuinely necessary.

Include:

- Original application icon.
- Source application name when useful.
- Notification title.
- Short body text.
- Relevant actions when available.

Notifications should appear temporarily and collapse after an appropriate interval.

### Desktop Notification Example

A development notification may display:

- Source: GitHub or a local development application.
- Title: Pull request merged.
- Body: Pull request #482 was merged into main.

The notification appears below the media player without changing its position.

### Android Notification Example

An iFood notification may display:

- Original iFood icon.
- Order status.
- Short notification text.
- Source Android device.

When reliable structured information is available, a compact progress indicator may represent:

- Confirmed.
- Preparing.
- On the way.

Do not invent progress stages or claim access to an application's internal order-tracking data.

If only notification text is available, display the notification text.

App-specific progress components require reliable source information and must not misrepresent the current activity.

### Instagram and TikTok

Social notifications should preserve their original application identity.

Example content includes:

- Instagram Close Friends story notification.
- TikTok new video notification.

Display thumbnails only when genuinely provided by the source notification or explicitly authorised integration.

Do not scrape private social content to recreate the mockups.

### Notification Centre

Provide an independently accessible right-side panel.

Include:

- Notification history.
- Paired Android connection status.
- Application identity.
- Source device identity.
- Notification timestamps.
- Unread indicators.
- Dismiss actions.
- Clear-all action.
- Supported contextual actions.

Group notifications by source, application or recency where appropriate.

The Notification Centre should remain closed during ordinary desktop use unless explicitly opened.

Displaying an incoming Island notification must not automatically open the entire Notification Centre.

### MonoApp Integration

MonoApp is a lightweight Android companion, not a messaging or social application.

It should provide:

- Authenticated pairing.
- Encrypted communication.
- Notification forwarding.
- Relevant activity updates.
- Device connectivity state.
- Explicit disconnect and revocation.

Respect Android notification permissions and privacy controls.

Do not require unrestricted access to the device.

Do not introduce mandatory cloud infrastructure.

### Privacy

Allow sensitive notification content to be hidden when appropriate.

Do not expose private content on the Windows lock screen by default.

Do not store notification history longer than required by the configured retention policy.

Do not duplicate native notifications unnecessarily.

Preserve native notification delivery when Monolith integration fails.

## 12. Control Center

The Control Center follows the same visual language as the approved desktop surfaces.

Its final detailed visual reference remains subject to founder approval.

Position the panel near the upper-right status area.

Prioritise compact controls for:

- Wi-Fi.
- Bluetooth.
- Brightness.
- Audio volume.
- Audio output.
- Battery.
- Focus modes.
- Notification preferences.
- Media controls when useful.

Use restrained blue highlights for enabled or selected controls.

Prefer functional controls over shortcuts to Windows Settings.

Clearly distinguish unavailable functionality and compatibility fallbacks.

Do not create oversized decorative tiles.

The panel must close naturally when dismissed or when focus moves elsewhere.

## 13. Animation and Interaction

Animations should communicate transitions and spatial relationships.

Use animation for:

- Island expansion and collapse.
- Notification appearance and dismissal.
- Spotlight opening.
- Dock hover and selection.
- Workspace transitions.
- Mission Control.
- Quick Look.
- Control Center appearance.

Avoid:

- Permanent decorative motion.
- Continuous background animation.
- Excessive bouncing.
- Long interaction delays.
- Unnecessary particle effects.
- Heavy real-time blur.

Suggested transition durations:

| Interaction | Duration |
|---|---|
| Hover feedback | 80–120 ms |
| Notification appearance | 120–200 ms |
| Spotlight opening | 120–200 ms |
| Island expansion | 150–250 ms |
| Workspace transition | 180–300 ms |
| Panel dismissal | 100–180 ms |

These are initial refinement targets and may be adjusted after actual interaction testing.

Prefer refresh-rate-aware animations and respect reduced-motion preferences.

## 14. Native Rendering

Monolith must preserve its native Rust architecture.

Use appropriate Windows rendering technologies:

- Win32.
- GDI where sufficient.
- Direct2D when justified.
- DirectWrite for improved typography when necessary.
- DWM for supported window composition and previews.

Do not introduce:

- Electron.
- Chromium.
- Embedded WebView runtimes.
- Browser-based rendering for native desktop components.

Vector iconography, smooth geometry and visual consistency must be achieved through native rendering.

Transparency and blur are optional implementation details.

Prefer efficient opaque or lightly translucent surfaces over expensive continuous visual effects.

Visual similarity to the approved references must not require excessive resource consumption.

## 15. Performance and Accessibility

The interface must remain responsive on the founder's existing hardware.

Maintain:

- Event-driven updates.
- On-demand rendering.
- Bounded background workloads.
- Lazy initialisation.
- DPI-aware layout.
- Keyboard navigation.
- Accessible control names.
- Visible focus indicators.
- Adequate text contrast.
- Reduced-motion compatibility.
- Proper native resource ownership.

Stop animations and timers when their associated interface is inactive.

Release previews, artwork and temporary surfaces when no longer required.

Do not sacrifice functional correctness for visual similarity.

## 16. Development Phases

### Phase 1 — Functional Daily Driver

Complete and stabilise the agreed personal desktop experience.

Temporary native controls are acceptable.

Preserve the approved layout, component responsibilities and interaction model.

Do not prematurely generalise the product for hypothetical community requirements.

### Phase 2 — Visual Refinement

After the functional experience is sufficiently stable:

- Replace temporary icons.
- Standardise typography.
- Unify geometry and spacing.
- Refine native rendering.
- Implement purposeful animations.
- Match the approved visual references.
- Resolve inconsistencies between components.

### Phase 3 — Performance Validation

Measure the completed visual experience.

Compare idle and active workloads.

Investigate regressions introduced by visual refinement.

Preserve the native performance objectives established in AGENTS.md.

### Phase 4 — Community Readiness

Begin only after founder approval.

Expand customisation, accessibility, compatibility and distribution without compromising the established product identity.

## 17. Visual Acceptance Criteria

A component is visually complete when:

- It follows its approved reference.
- It uses the shared design system.
- Its information hierarchy is clear.
- Its icons are consistent.
- Its geometry and spacing are intentional.
- Its interactions are predictable.
- Its animation is purposeful and responsive.
- It displays real runtime information.
- It handles missing or unsupported information gracefully.
- It preserves native performance.
- Its appearance has been verified through actual Windows screenshots.

Compilation success does not establish visual completion.

A generated concept image does not establish implementation completion.

Do not report a component as visually complete until its actual rendered appearance has been reviewed.

## 18. Final Product Direction

Monolith must feel like a coherent desktop environment rather than a collection of independently styled utilities.

The Dynamic Island, Dock, Spotlight, Files, Workspaces, Control Center and Notifications must share a recognisable visual language whilst remaining independently functional.

The approved references establish the destination.

Functional correctness establishes the foundation.

Native performance remains a non-negotiable engineering requirement.

The final experience must deliver the freedom of a deeply customisable Linux desktop without sacrificing the reliability, software compatibility and update experience of Windows.