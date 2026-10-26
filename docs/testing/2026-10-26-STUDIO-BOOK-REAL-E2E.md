# Studio–Book real-device acceptance checkpoint (2026-10-26 UTC)

This checkpoint used the installed Weavelo Desktop applications, not a test
mock. Studio ran the package built from `b79e3b9`; Book ran `e4a57619`.
Both advertised the same device-group network-name SHA-256
`879e2e8a092f80fe0e8770f5df795bbf4718e68abcf343c85cef3c8f433ca090`.
No group membership or credential was changed.

Studio's real GUI displayed `MBPM2MAX.local` as an available relay peer.
After **Connect Stream**, the existing read-only GUI diagnostic reported
`connected=true`, `peer_responsive=true`, `route=Relay`,
`confirmed=false`, `decoded_frames=0`, `decode_errors=0`, and
`video_error="subscription is unavailable"`. The same result persisted after
waiting and on a second controlled attempt. There was no captured, decoded,
or presented video frame. Book's read-only GUI network diagnostic reported
both relay channels connected; this does not establish video acceptance.

The native **Files → Send files** path did complete one real transfer. Studio
sent only `/tmp/weavelo-e2e-20261026-studio-to-book.txt` (58 bytes, synthetic
content). The GUI showed 100%. An independent Book read of
`/Users/jinliang/Library/Application Support/Weavelo/Desktop/Received Files/weavelo-e2e-20261026-studio-to-book.txt`
confirmed 58 bytes and SHA-256
`547b264c333222731831f715b91364e336faafb693b0147272ffb270ea2673ff`,
matching the Studio source. Octendra terminal operation
`1fdf94c8-a296-400e-a20d-a6b15f207aef` is the receiver-side readback
receipt; it exited 0 at 2026-10-26 16:35:40 UTC. The transfer was not
replayed. This proves this one file direction, not reverse transfer or video.

The Book→Studio file path, reverse video, input, audio, and long-duration
behavior remain unverified. The later Computer Use attempt to create a
synthetic test window was interrupted by the user. No further GUI action was
taken; the existing file-only GUI session was subsequently observed read-only
as still connected and responsive.

## First-failure evidence gap

The installed Book host process (`61162`) was running and bound UDP `*:39273`;
its relay TCP connections were established. Book's installed app passed strict
`codesign` verification with bundle ID `fun.hackerlife.weavelo.desktop` and
designated leaf SHA-1 `8eaa97365d97b8a987cdf1232430eddf9aba8ecf`.
The clamshell was open and macOS reported two online displays. These facts do
not prove ScreenCaptureKit authorization or successful capture startup.

The current host source logs an asynchronous capture failure to stderr and
sends it as a session error (`host/src/service.rs`,
`Capture subscription {id} failed: {error}`). Book's LaunchAgent has neither
`StandardErrorPath` nor `StandardOutPath`; `WEAVELO_SESSION_TRACE` is absent
from its plist. The targeted project log directories were absent and a
unified-log query for host PID `61162` around the attempt returned
no events. The GUI status exposed the final `subscription is unavailable` but
not the preceding error, and the original video session was closed. No
retained host error, original video connection ID, or subscription-event
timeline exists for these two attempts. The current source's cleanup error
can reuse the video subscription ID and mask the first failure; commit
`02adb3a` prevents that masking for a future compatible build.

The minimum next real-device check, after a GUI session resumes and the
compatible fixed client is installed through the normal signed updater, is
one Studio→Book **Connect Stream** attempt. Capture the first non-secret
`video_error`, `connection_id`, `subscription_id`, confirmation and decoded
frame counts from the existing read-only GUI diagnostic, then stop that
specific attempt. A capture/startup error would direct investigation to the
Book host's ScreenCaptureKit/encoder path; a subscription confirmation timeout
would direct it to host control delivery and cancellation order. Neither
branch is proven by the old final error alone.
