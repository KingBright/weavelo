# Background status observation boundary

`--background-readonly-status-json` is a source-only candidate. It has not been
installed on HO5 and must not be used as a substitute for the previously denied
production `--background-status-json` request without a separate review.

The new CLI branch runs before `UnifiedRuntimeConfig::from_env`. It resolves only
the existing Desktop profile path and calls `background_service::read_only_status`.
That function hashes its own executable and exchanges one `ReadOnlyStatus` frame
over the existing private same-user IPC transport. It never constructs `Client`,
loads the mesh store, opens a recovery file, initializes preferences, or prints a
pairing snapshot. A retained-client profile is rejected.

The background owner returns only schema, role, version, generation, PID,
Linux process start ticks, executable SHA-256, and an advisory `idle_now` boolean.
The CLI is now Linux-only. It checks kernel `SO_PEERCRED` PID/UID and reads the
peer's `/proc/<pid>/stat` start ticks and `/proc/<pid>/exe` running image before
and after the IPC exchange. The response must match that OS tuple and the
caller's exact executable hash. A missing `/proc` observation fails closed;
macOS and Windows return unsupported rather than relying on socket pathname
metadata as process attestation. The owner reads role, transfers,
delivery tickets, and host connection/handshake state. It neither expires
handshakes nor requests a stop. Missing host state, protocol/version mismatch,
wrong role, or executable mismatch fails closed. The later idle-only stop still
rechecks all owners; this observation alone never authorizes an upgrade or move.

Focused review found no new credential export or logging path. The IPC transport
still authenticates the same local user and checks the protected endpoint. Tests
use only synthetic recovery bytes to assert that a query preserves content,
inode, timestamps and permissions, and that the response omits those bytes. They
also reject alternate protocol versions, roles, generations, hashes and extra
response fields. A same-user process with control over another process remains
outside this operational identity check's threat model. Real HO5 execution,
deployment, group backup, invitation
transfer, and cross-device acceptance remain separate gates.

The following Studio main-checkout targeted readback was for the initial
`1116080` implementation, before Linux peer/start-time binding was added:

- `cargo test -p weavelo_app --lib background_service::tests::read_only_status`:
  2 passed, 0 failed. This includes a private IPC fixture with an invalid
  synthetic pending recovery record that is preserved byte-for-byte and in
  inode, timestamps, and mode.
- `cargo test -p weavelo_app --lib
  background_service::tests::frontend_is_not_a_second_network_owner_and_outlives_only_its_windows
  -- --exact`: 1 passed, 0 failed. This uses the actual local background owner
  and verifies the same pending-record invariant through its response branch.
- `cargo test -p host --lib upgrade_stop_refuses_all_active_owners`:
  1 passed, 0 failed. A preceding `--exact` invocation matched 0 tests and
  is not counted as validation.

The shared Cargo lock was observed released after those tests. The tightened
Linux-only implementation and its new CLI, live host-observer channel, and
forged-peer tests are source-prepared. After the tightening, Studio ran the
platform-neutral version/identity rejection test (**1 passed**) and the new
synthetic host-observer channel test (**1 passed**). The Linux-only peer,
pending-recovery, live-host and CLI subprocess tests were conditionally absent
on Studio and are **not** counted. An `x86_64-unknown-linux-gnu` Cargo check
remained in dependency fetching without creating target build output; its
original process was ended, exit 130, and the shared lock was observed free.
The tightened Linux implementation therefore has no native compile or runtime
acceptance yet and is not authorized for production use.
