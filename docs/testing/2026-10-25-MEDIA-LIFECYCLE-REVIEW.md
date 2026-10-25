# Media lifecycle review, 2026-10-25

Reviewed the primary `main` checkout at `4d41b3c7890ec29ab0845bbc362f9c7a0777a852`.
The worktree was clean before this change, with no second source checkout or
Weavelo producer observed. A later Forge test held the shared Cargo build lock;
Weavelo validation waits for that lock and does not stop the other producer.

## Existing implementation and remaining scope

| Boundary | Current source evidence |
| --- | --- |
| Consumer demand | `remote_core/src/media_demand.rs` merges video/audio independently and fences replaced consumer callbacks by generation. The desktop model uses this registry. |
| Retained transport | Desktop demand changes use the existing `SetActivity` revision and acknowledgement path; they do not close the workspace connection. |
| Recovery | `video_recovery.rs`, workspace ingress and client payload-boundary tests already fence stale sources/decoder attempts and resumed packets. |
| macOS capture | Video and system-audio `pause` retain the SCStream owner and discard obsolete payload; actual stop remains separate. This is source evidence, not a new physical measurement. |
| Linux/Windows encoders | Existing pause branches still release the raw/native encoder or FFmpeg source and recreate it on resume. Cross-platform retained-encoder acceptance is incomplete. |
| Service ownership | `app/src/service_contract/session.rs` explicitly remains an executable adapter contract. The GUI session still owns receiver resources and unsubscribes on Drop; core-service migration is not complete. |

The 2026-10-14 plan's dated “not implemented” status must not be interpreted as
absence of the later partial implementation. This review does not authorize or
perform a new service architecture, native-resource transport, or deployment.

## Targeted correction

When a background consumer kept both media payloads active, deselecting a GUI
view left aggregate demand unchanged. The desktop model therefore skipped the
activity transition that otherwise released held input, and its input predicate
did not independently require selection.

The model now releases that view's held input on deselection and requires the
view to be selected before admitting input. It preserves the other consumer's
payload demand, activity revision and connection. The regression exercises the
actual desktop model and a loopback workspace actor using synthetic paint
eligibility, without a native window, screen capture or OS input injection.

## Validation

- `PYTHONDONTWRITEBYTECODE=1 ./scripts/test_release.sh`: 805 tests, successful,
  23 conditionally skipped. The final run uses the canonical existing temporary
  directory and ordinary process observation; signed integration is not enabled.
- `computer-use-input`: the guarded desktop-model build reached its existing
  1800-second deadline after shared-cache waiting and dependency compilation.
  Exit 124, zero tests executed, source hashes unchanged, post-observation exit
  zero and owned process group confirmed absent. This is not a passing Rust test.
  Receipt: `.rust-storage/validation-logs/computer-use-input-ui0ed0_a.jsonl`.
- `clock-workspace`: 25 passed, zero failed/ignored; source hashes unchanged,
  post-observation exit zero and owned process group confirmed absent.
  Receipt: `.rust-storage/validation-logs/clock-workspace-zj3qf9hq.jsonl`.
- `media-lifecycle-boundaries`: cancelled while Cargo was still waiting behind
  Forge Clippy (PID 27051) and another Forge test. Exit 130, zero tests executed,
  source hashes unchanged, post-observation exit zero and owned process group
  confirmed absent. No other producer was stopped or cache copied.
  Receipt: `.rust-storage/validation-logs/media-lifecycle-boundaries-undt2qxm.jsonl`.

### Follow-up after the shared target became idle

The two unfinished modes were resumed with their original 1800-second build and
120-second test deadlines, using the same cache after observing that Forge had
released it. The previous 805-test fixture suite and 25 core session tests were
not repeated; their scopes were unchanged.

The first desktop execution compiled successfully but reported two passes and
three fixture failures before the target assertions. The recorder advertised
`files=false` but used `WorkspaceConnection::connect`, which correctly rejected
an unpaired native file session. The test-only fixture now uses the existing
`connect_media` API and asserts that file capability is unavailable and its file
command channel is closed. No production authentication rule, device group,
permission or connection API was changed.

- Failed fixture receipt: `.rust-storage/validation-logs/computer-use-input-holpd1a0.jsonl`.
- Corrected `computer-use-input`: **5 passed, zero failed/ignored**, including
  `deselected_view_releases_input_while_another_consumer_keeps_payload_active`.
  Receipt: `.rust-storage/validation-logs/computer-use-input-lrspctfj.jsonl`.
- `media-lifecycle-boundaries`: **3 passed, zero failed/ignored**: retained resume
  IDR, watch/packet-ready ordering, and pause/resume through a full audio queue.
  Receipt: `.rust-storage/validation-logs/media-lifecycle-boundaries-59pp0_wf.jsonl`.

Both successful follow-up receipts confirm unchanged source hashes, successful
post-observation and absence of their owned process groups. They validate the
desktop model and synthetic payload queues, not cross-platform native resource
retention or physical presentation.

## Deployment assessment

Not eligible for deployment from this run. The canonical
`package/macos/Weavelo-macos-arm64.zip` was absent when checked, and this run did
not create a formally observed build-identity receipt or an authorization-compatible
signed package for the changed source. Library test compilation cannot replace
the existing identity build, production packaging/signing and compatible install
plan. No packaging, publication or installation was performed.

Physical acceptance remains separate: two-Mac pause/resume resource identities,
fresh actual presentation and reliable input, controlled audio without replay,
long-duration behavior, and native HO5/cube behavior have not been measured in
this run. No installed application or published package is changed.
