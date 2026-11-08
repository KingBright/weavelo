# Remote-view network status

The remote-control canvas keeps a compact network-quality indicator visible after its tools hide. The management screen is unchanged. The indicator starts collapsed; a click pins a small details panel, and mouse hover reveals details without pinning. Closing the panel returns to the compact indicator; desktop Escape also closes it.

The indicator uses the current session RTT, with existing desktop quality thresholds and matching Android latency/loss thresholds. Details show RTT, packet loss and bitrate. Missing or expired measurements render as unavailable, and connecting/reconnecting/disconnected sessions never reuse an old healthy value. Android records the monotonic observation time of a telemetry read and expires its display after five seconds. Zero frame rate does not imply a poor network, and neither decoding nor these statistics imply physical presentation.

The overlay does not reserve video layout space or change the video/input transform. Desktop opening the panel releases held remote input and focuses the local UI. No new network probes, connection mutation or dependencies are introduced. Chinese and English labels are included. Android build number advances to 112.

Validation and actual rendered acceptance are recorded separately in the task evidence. No cross-platform physical acceptance is implied by source tests.

Source verification: nine existing desktop telemetry tests passed; all 104 Android JVM tests passed (including fresh/missing/reconnecting/latency/loss cases); the release regression entry passed 1053 tests with 25 environment skips. Actual rendered acceptance and deployment remain pending at this source checkpoint.
