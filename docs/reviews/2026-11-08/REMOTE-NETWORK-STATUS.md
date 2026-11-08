# Remote-view network status

The remote-control canvas keeps a compact network-quality indicator visible after its tools hide. The management screen is unchanged. The indicator starts collapsed; a click pins a small details panel, and mouse hover reveals details without pinning. Closing the panel returns to the compact indicator; desktop Escape also closes it.

The indicator uses the current session RTT, with existing desktop quality thresholds and matching Android latency/loss thresholds. Details show RTT, packet loss and bitrate. Missing or expired measurements render as unavailable, and connecting/reconnecting/disconnected sessions never reuse an old healthy value. Android records the monotonic observation time of a telemetry read and expires its display after five seconds. Zero frame rate does not imply a poor network, and neither decoding nor these statistics imply physical presentation.

The overlay does not reserve video layout space or change the video/input transform. Desktop opening the panel releases held remote input and focuses the local UI. No new network probes, connection mutation or dependencies are introduced. Chinese and English labels are included. Android build number advances to 112.

Validation and actual rendered acceptance are recorded separately in the task evidence. No cross-platform physical acceptance is implied by source tests.

Source verification: nine existing desktop telemetry tests passed; all 104 Android JVM tests passed (including fresh/missing/reconnecting/latency/loss cases); the release regression entry passed 1053 tests with 25 environment skips. Actual rendered acceptance and deployment remain pending at this source checkpoint.

The Android workspace remote view also exposes the same collapsed indicator for each active pane. Its native subscription measurements are sampled once per second, expire at the source, and reset on reconnection. The bridge missing-measurement regression and Android debug unit suite pass; the release regression remains 1053 passing tests with 25 skips.

Native MacStudio acceptance: signed desktop source 1f367145e220043d0404bec7978b3192483536aa installed through the existing compatible desktop upgrade entry. Receipt phase completed; running frontend PID 46704 and background PID 46698 matched executable e67756ec7319bba1fb7823daf1f1b64a0f8bd9f48198047eb7f4c115da977c82. Membership, launch policy and prior rollback slot were preserved. Actual MBPM2MAX Display 1 video showed the collapsed indicator while tools were hidden; clicking opened RTT/loss/bitrate details, Escape closed the panel, and the pointer remaining over the chip showed the hover tooltip. Observed sample: 51 ms RTT, 0.0% loss, 107 kbps. Management had no added status chip. This covers the observed windowed Mac session only, not every DPI/platform or long-duration acceptance.

Android delivery remains blocked by the release lifecycle rejecting unrelated uncommitted inputs protocol/src/audio_output.rs and remote_core/src/audio_output.rs. Those files were preserved; no Android installation or NAS publication is claimed for build 112.
