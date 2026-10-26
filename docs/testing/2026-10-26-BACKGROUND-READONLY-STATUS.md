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
executable SHA-256, and an advisory `idle_now` boolean. It reads role, transfers,
delivery tickets, and host connection/handshake state. It neither expires
handshakes nor requests a stop. Missing host state, protocol/version mismatch,
wrong role, or executable mismatch fails closed. The later idle-only stop still
rechecks all owners; this observation alone never authorizes an upgrade or move.

Focused review found no new credential export or logging path. The IPC transport
still authenticates the same local user and checks the protected endpoint. Tests
use only synthetic recovery bytes to assert that a query preserves content,
inode, timestamps and permissions, and that the response omits those bytes. They
also reject alternate protocol versions, roles, generations, hashes and extra
response fields. Real HO5 execution, deployment, group backup, invitation
transfer, and cross-device acceptance remain separate gates.

Studio main-checkout targeted readback on the shared `~/rust-target` cache:

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

The shared Cargo lock was observed released after these tests. These checks
do not establish Windows/Linux native behavior or authorize production use.
