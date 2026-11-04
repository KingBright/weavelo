# Native remote interaction

Remote applications should be recognizable and behave predictably as local
windows. Keep common session, input and geometry logic, with native media and
OS adapters. Measure performance on the actual machines and both USB phones.

The next delivery phases for active push, a pushable whiteboard, and teaching
cursors/annotations are tracked in [the cross-device plan](ACTIVE-PUSH-WHITEBOARD-TEACHING.md).

## Window identity and controls

Show the confirmed application/window and device in the OS window title and
the compact toolbar. Use the real application icon from the confirmed source
in the native receiving window and PiP; update it when the source or artwork
changes and retain the distinct remote-window mark for other sources.
Fullscreen/restore and the system titlebar controls operate on the local viewer;
minimizing the controlling viewer releases remote input. Local window shortcuts
are handled above the focus scope, including idle and drawer focus; remote keys
still require the confirmed video input focus.

The approved source catalog carries application names, titles and bounded
64 × 64 / 16 KiB PNG icons. Cache these outside the frame path; native titlebar
artwork updates only when the selected asset changes. Do not
fetch icons from external URLs or place image work in the video loop.

On macOS, custom titlebars may expose a separate native traffic-light window
at a non-default inset. Verify its exact owning key window through Accessibility;
do not infer keyboard ownership from WindowServer order alone. Ordinary dialogs,
menus, cross-process occlusion and pointer hit checks remain enforced. Scroll
events carry their own inverse-transformed pointer location through to the OS.

## A single geometry model

1. Obtain the codec's visible dimensions, excluding inclusive crop padding.
2. Fit that rectangle inside the available content area after system bars,
   keyboard, local controls and panel layout have been accounted for.
3. Apply zoom and pan to the native video surface. Invert that exact transform
   for direct touch; trackpad movement remains relative.
4. Send normalized coordinates. The host maps them to the selected source's
   current OS coordinate space, including monitor origin and OS DPI/scale.
   Encoded pixels must not be confused with macOS points or Windows DIPs.
5. On geometry, source, surface or session change, cancel the previous gesture.
   Its cleanup retains its original owner. It cannot release a new owner's
   gesture. Keep the original Android event timestamp through the serial queue.

The Android single-view and workspace paths now use the same production
viewport and gesture implementation. Workspace resolution negotiation measures
the fitted video rectangle, not the entire portrait panel, and checks the
session/generation before applying an asynchronous resize.

## Keyboard and system behavior

Keep IME candidate composition local; transmit committed input once. Preserve
selection, delete, Enter, shortcuts and key release as separate intentions.
Keyboard visibility changes available layout, not remote coordinates. A drag
in progress must be cancelled during the resize; after the animation, taps map
against the newly measured video rectangle.

The approved committed-text channel carries up to 1 KiB UTF-8 per event through
the current authenticated source/input owner and native host injection. Physical
keys retain shortcuts and releases; IME composition stays local. Do not emulate
text by silently changing the user's clipboard. Actual soft-keyboard candidates,
OEM input methods and each platform's application focus still need physical
acceptance; a synthetic commit alone does not cover all IME behavior.

Android targets SDK35: handle edge-to-edge and IME insets through Compose's
inset-aware layout, without subtracting keyboard height again in input code.
Windows must handle per-monitor DPI changes. Linux Wayland capture/input uses
the compositor's actual portal consent. macOS signing compatibility preserves
identity but does not grant new capture/input permissions.

## Performance and verification

Retain CVPixelBuffer/native textures, Windows native resources, Linux native
buffers and Android MediaCodec-to-Surface. Avoid CPU video readback/conversion,
per-frame icon work and duplicate session/gesture state machines. Negotiate
only useful visible resolution; do not encode black margins or keyboard area.
Keep bounded media queues and preserve compressed reference frames.

Verify an owned grid/text fixture at center and corners with fit/fill, zoom/pan,
window/fullscreen/PiP, portrait/landscape, IME shown/hidden and representative
DPI. Read independent fixture effects for click, drag, scroll, text and releases.
Record frame presentation, errors, latency, CPU, GPU/native copies and memory;
compilation, decoder output and a source list do not complete this acceptance.
Use main, focused checks, the release regression, commit/push and compatible
updates. An observed failure triggers a direct fix, not another release layer.

References: [Android insets](https://developer.android.com/develop/ui/compose/system/insets),
[Compose inset layout and IME animation](https://developer.android.com/develop/ui/compose/system/insets-ui),
[Windows DPI changes](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows).
