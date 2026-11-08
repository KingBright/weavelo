# Chinese and English interface

The desktop Settings drawer, Android Settings and Web connection panel now expose self-named `中文` / `English` options. The initial language follows the operating system (Chinese locales select Chinese; other locales select English). Explicit selection persists locally and refreshes open interface content without restarting the network session.

Localization covers application-owned navigation, stream controls, source selection, pairing, device management, sharing permissions, file transfer, telemetry, menus, dialogs and status messages. Desktop uses a shared catalog and Rust format checking; Android uses a shared Compose catalog and preference-backed notification strings. The Web console persists its preference in browser storage and refreshes existing status, title and accessibility labels without reconnecting. Device, application and file names, paths, identifiers and opaque operating-system diagnostics retain their original content. OS-owned permission and file-picker chrome follows the operating system.

Language is UI preference only. Existing configuration merge/locking, network identity, media paths, input ownership and signing requirements remain unchanged.

Validation results and device acceptance are reported separately. Source tests do not establish installation or native visual acceptance on any platform.

Validated on the primary macOS checkout: 75 focused Rust tests passed (one existing preference test ignored); Android compilation and 101 JVM tests passed, and release instrumentation tests compiled; Web tests passed 14/14; release regression passed 1053 tests with 25 environment skips. Desktop literal translation-key coverage has no missing catalog entries, and all catalog format-argument multisets match. Native Windows/Linux compilation and physical language switching, notifications, layout and live-session acceptance remain unverified for this iteration. No package or installed application has been updated by this source iteration.
