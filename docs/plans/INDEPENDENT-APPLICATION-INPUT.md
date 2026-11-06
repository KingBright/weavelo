# Independent application-window interaction

User clarification, 2026-11-06: two separately streamed application windows must
remain independently usable when the user alternates between them. Strictly
simultaneous input is not required. Remote stacking order and occlusion should
not make either viewer pause control, lose typing, or receive the other's input.

## Acceptance

- Open two application sources without opening a desktop stream. Cover A with B
  on the host, then alternate operations through their separate viewers.
- Each viewer retains its own pointer coordinates, scroll state and selected
  source. Text, Enter, selection and dragging reach the intended window.
- Repeat for different processes and two windows belonging to one process;
  process-targeted events alone cannot prove exact-window routing.
- Hold/release modifiers and drag across a viewer focus transition. No stuck
  keys, cross-window releases, duplicated text or control-paused toolbar.
- Check native editors, browser/Electron content and custom-drawn content;
  successful accessibility actions on one editor are not universal support.
- Inspect actual target content and the other window's unchanged state.
  Decoding a frame or accepting an input message is not input acceptance.

## Current verified gap

The macOS application input adapter posts global HID events and requires native
foreground/window hit confirmation (`host/src/input_injector.rs`,
`host/src/window_input.rs`). Its current CUA entry shares input infrastructure;
the CUA label does not provide independent background application input.
Book's application-stream observation reported an occlusion error followed by
control-paused state. Automatically activating the application before a wheel
event improves the existing foreground path but does not meet this requirement.

## Implementation investigation

Keep authenticated source identity, current source geometry and per-session input
state shared. Evaluate native process/window-targeted events and accessibility
actions on owned fixtures before selecting an adapter. Preserve input-method
composition, shortcuts, scrolling and cancellation semantics; setting an entire
text field's value is not equivalent to typing.

Apple exposes process-targeted CGEvent posting and accessibility object actions.
Neither API contract guarantees arbitrary exact-window background interaction:
validate same-process windows and unsupported controls explicitly. Do not remove
the existing global HID hit guard or silently redirect a rejected operation.

If a platform needs foreground activation, report that limitation accurately.
Automatic activation may be an incremental improvement, but is not independent
background input and must not be reported as completion of this requirement.

References:
- https://developer.apple.com/documentation/coregraphics/cgevent
- https://developer.apple.com/documentation/applicationservices/1462091-axuielementperformaction

Status: foreground switching verified on owned native fixtures; general background interaction remains unimplemented.
No claim of implemented or deployed independent background input.

## Native foreground-path verification, 2026-11-06

The macOS adapter now waits for the selected pointer hit target rather than only
the application's frontmost flag. Fresh key/text input can activate the selected
window without synthesizing an editor click. An AX window is raised only when its
owner and unique live geometry identify the selected WindowServer ID. Existing
deadlines and final target checks remain; releases do not activate old targets.

An explicitly invoked native test exercised the actual adapter against two owned
AppKit editors: different processes with overlapping windows, then two windows
in one process. A/B/A/B committed Chinese, emoji and Return were independently
read back from each editor. Both runs passed; the second visit to each editor
used keyboard input without another mouse click. Evidence is in the task's
`native142-input-acceptance` directory (`native-test-fad4*` and `native-test-daeef*`).
This proves the native adapter's foreground switching, not cross-device delivery,
unchanged host stacking order, arbitrary applications, or glass-to-glass latency.

The fixture requires a real NSTextStorage/LayoutManager/Container. The earlier
background probe's nil text container could not verify typing. Also, run the
native test binary through a small owned app bundle: CoreFoundation's initial
bundle search in the large Cargo dependency directory added seconds unrelated
to the installed app. Keep these failed attempts distinct from product evidence.


## Controlled background comparison, 2026-11-06

After a successful real-editor HID baseline, a second owned application covered
the same target point. Process-targeted events constructed through AppKit, with
Quartz screen coordinates and public window fields, produced no target callbacks
and no editor text change for click, scroll, committed Unicode, or Return. The
covering editor also received no callbacks. Read-only snapshots confirmed the
unchanged text. Evidence: task `native142-input-acceptance`, report
`native-posttopid-5d4c600380454d8ab0dde2eafd13a47b-REPORT.md`.

This rules out that tested process-posting path as a working implementation on
this machine; it is not a proof that every macOS background technique is impossible.
Do not ship an event-post success as successful background control. The tested
foreground adapter remains the current working path. Exact-window accessibility
or application-specific adapters need independent control-level acceptance before
claiming equivalent text, scrolling, composition, or shortcuts.
