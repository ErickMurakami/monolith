# Changelog

## Unreleased

- Added the native Spotlight launcher for Start menu shortcuts.
- Added local configuration, shortcut conflict handling and safe start.
- Added Windows CI for build, lint and test validation.
- Added reproducible Windows and launcher performance sampling.
- Added local arithmetic in Spotlight, with precedence and input error handling.
- Added title search and switching for open windows, with monitor and fullscreen detection.
- Added an opt-in native Dock with live window titles, minimisation, restoration and fullscreen suppression.
- Added ordered Dock pins from the shared Start menu shortcut catalogue.
- Refined Spotlight into a borderless, DPI-aware overlay that dismisses on Escape or loss of focus.
- Added drag reordering of Dock pins and local order persistence.
- Grouped Dock windows by application identity and added a native window chooser.
- Added an opt-in borderless Top Bar with the native clock and battery status.
- Added active-display Dock placement and an optional pointer reveal strip.
- Added a registered fallback for reserved Spotlight shortcuts, press-to-toggle and foreground restoration.
- Added live DWM thumbnails for Dock window tiles on pointer hover.
- Added cached monochrome Shell icons and native tooltips to the Dock.
- Added a native Dynamic Island for Windows media metadata and transport actions.
- Added playback progress based on the Windows media timeline.
- Added artwork loaded on demand from the active Windows media session.
- Added the borderless Control Center with native volume and mute, battery status and Windows settings shortcuts.
- Added a Mission Control window overview with live thumbnails, mouse and keyboard selection, and a native Task View action.
- Added an on-demand Monolith Files window with virtualised directory listings, history, standard folders and native app activation.
- Added native Quick Look for text, Markdown, images, first-page PDF previews and basic item details.
- Added Files rename and folder creation with native collision handling.
- Added directional switching between existing Windows desktops in Mission Control.
- Added on-demand Git branch, upstream and file status in Monolith Files.
- Added terminal and configured IDE actions to the Files context menu.
- Reserved native work area for the Top Bar so maximised window captions stay accessible, and added the local date.
- Added Files copy, cut and paste through the Windows clipboard and Shell, with collision-safe naming and cancellation requests.
- Added recycling from Files through the Windows Shell, without a permanent deletion path.
- Pinned Dock tiles adopt their running application windows and show running indicators.
- Added an opt-in Win+Space bridge for Spotlight that keeps keys typed before activation.
- Shift+click on a Dock pin starts another instance.
- Added opt-in Win+1-9 workspace selection and Win+Tab Mission Control through the keyboard bridge.
- Added an experimental Dock taskbar replacement mode with session-only visibility and native recovery.
