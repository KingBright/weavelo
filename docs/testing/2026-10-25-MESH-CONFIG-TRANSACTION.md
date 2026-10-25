# Per-profile device-group save recovery (2026-10-25)

Status: source-only implementation and isolated validation. No real invitation,
credential, profile, group membership, installed executable or OS permission was
changed during this work. The user's 08:28:51 UTC approval covers the concrete
protected recovery-copy scheme and a later HO5 move after its gates pass; approval
is not proof that the move occurred.

## Storage boundary and commit

The existing production backend is `AppPrivateMeshSecretStore` (private credential
files), not an OS keyring implementation. The recovery slot is
`mesh.secret-recovery`, in the same validated private profile directory and with
the same owner/symlink/hard-link/mode checks as `mesh.secret` (Unix directory 0700,
files 0600 by default). This change adds no encryption; protection remains the
existing credential backend's owner/private-directory/file boundary. No key is exported into a log, receipt, public metadata,
invitation argument, ordinary staging file or public journal. This adds no device
access grant, OS credential permission or persistent authentication identity.

`MeshRecoveryRecord` is opaque and redacts Debug output. It holds exact before and
after public metadata and credentials only inside that credential backend. A
secure-store adapter must implement recovery in its own protected backend; the
default refuses mutation before canonical writes and never falls back to file
storage. The mock keyring remains memory-only. Encoding is bounded and roundtrip
validated before persistence; parsing errors do not include payloads.

All participating profile operations use an exclusive nonblocking lock. Unix locks
the validated profile directory fd, including a dev/inode/owner/mode recheck after
acquisition; this preserves old two-file read-only profiles without adding a lock
file or widening access. Other platforms use the private profile lock file.

A write stages and independently reads the protected record, replaces the secret
and metadata, verifies the complete after tuple, then deletes the recovery record
as the logical commit point. File writes and Unix namespace operations are synced.
Native no-replace publication prevents the initial hard-link crash window. Join
preparation and saving share one lock and preserve the installation node ID,
display name and profile path. Ordinary valid reads preserve existing file bytes,
inodes and permissions, including legacy metadata.

Recovery accepts only the same profile binding and values matching the recorded
before/after tuple. It restores and verifies **before**, never replays the intended
join. A mutation with pending recovery refuses rather than recovering and then
repeating its invitation. Unknown, malformed, oversized or foreign state is
retained and refused. An interrupted first initialization is rolled back without
silently generating another identity in that startup call.

Errors after the logical commit are distinct from rollback failures:

- A matching retained record permits verified rollback and an ordinary failure.
- No record plus a complete after tuple yields `CommittedDurabilityUnconfirmed`.
  Desktop and Bridge must successfully re-read saved configuration, refresh the
  paired secret and request network reload, while reporting a warning.
- Unreadable or unmatched results yield `TransactionOutcomeUnknown`; do not replay.
- A stale desktop snapshot is reconciled even for an already-saved no-op join.
  Bridge retains an in-memory reconciliation flag across readback failures.
  A closed reload receiver returns an explicit saved-but-refresh-unavailable error
  and preserves that flag; successful queueing is not actual connectivity.

## Isolated evidence

All credentials are generated synthetic data. No real user credential was read.

| Entry | Result | Evidence |
| --- | --- | --- |
| `python3 -B scripts/test_stream_settings_model.py --mode mesh-profile` | 37 passed, 0 failed, 0 ignored | `.rust-storage/validation-logs/mesh-profile-5ozszib4.jsonl` |
| `TMPDIR=/private/tmp PYTHONDONTWRITEBYTECODE=1 ./scripts/test_release.sh` | 856 total: 833 passed, 23 skipped | `/tmp/weavelo-mesh-transaction-release-final.log` |
| `device-name-pairing` | 10 passed, 0 failed, 0 ignored | `.rust-storage/validation-logs/device-name-pairing-r60gfplo.jsonl` |
| `bridge-invitation` (Android feature enabled, synthetic fixtures only) | 6 passed, 0 failed, 0 ignored | `.rust-storage/validation-logs/bridge-invitation-5uqz19_c.jsonl` |

Core coverage includes write/create/fsync/publication/removal boundaries in
initialization, join, deletion and rollback; both returned errors and unwind
interruptions; restart recovery; concurrent read/write refusal; unknown write
outcomes; commit acknowledgement failures; foreign profile/unsafe recovery files;
size-limit refusal before any write; unsupported backend refusal; old read-only
profile preservation; node/name/profile preservation; and memory-only keyring
recovery with no file fallback. Test log interruption messages contain no payloads.

Independent read-only review identified and drove fixes for commit/runtime split,
record size poisoning, read-only legacy lock compatibility and Bridge readback
reconciliation. Final independent read-only review found no remaining concrete code blocker.
Caller tests above completed with source hashes unchanged and owned process groups
absent. Bridge compilation first found an exhaustive startup-diagnostic match
missing the new outcome variants; static nonsecret codes were added and the fixed
Bridge entry was rerun successfully. This is not a release or live-move receipt.

## Limits and actual-move gates

Unwind tests are not hardware power-loss tests. Windows namespace durability and
ACL equivalence, a native Windows secure store, actual platform crash recovery and
end-to-end membership/media/input/audio remain unverified. Windows directory sync
is not implemented here; do not represent the Mac/mock results as Windows native
acceptance or deploy Windows on that basis.

A process interruption can leave random private credential temporary files in the
same protected backend. The fixture verifies their owner/mode/link protection and
that unknown leftovers are retained. No broad cleanup is performed, and this work
does **not** claim that all crash remnants are removed. The formal recovery slot
is removed after a verified successful transaction/recovery; unknown temporary
contents stay protected pending explicit provenance-aware recovery.

The installed Studio build is still 20261025.9 (source d722c5a9), and the last
observed HO5 binary is from 26a9c0c. This source fix is not in those installations.
Carry it through the existing next normal release, signed identity gates,
installation receipts and rollback procedure; do not mutate an old versioned ZIP.

Before the authorized HO5 move, establish both:

1. A supported confidential source-to-target invitation channel without chat/log
   output, a credential-bearing terminal input, or ordinary-file staging. The
   available terminal-input tool explicitly forbids credentials; file upload/sync
   accepts regular files and is not an opaque invitation channel. A product-owned
   FIFO export is only a local endpoint, not proof of an approved cross-host route.
2. A verified recoverable source for HO5's **original Desktop group**. The transient
   transaction record is deleted at commit and is not a permanent old-group backup.
   An application rollback preserves an executable, not the old group credential;
   a different retained legacy profile is not the original Desktop group either.
   Zero online peers does not establish an authoritative empty membership roster.

Neither technical gate may be replaced by general authorization or a synthetic
fixture. Only after satisfying them and installing validated code may the actual
move be attempted once, followed by independent public identity/group readback,
mutual device discovery and real test-window video/input/audio acceptance.

## Validated source bytes

- `remote_core/src/mesh.rs`: `8f93c72360a4e7f46eb7bbb8e706aae02d9ddddef42f6753719b67c080523a76`
- `client/src/mesh_pairing.rs`: `d9e4afb7e7c84fa9057660faaa17e61cf06d2529551cc6ad66a8eddee3c25025`
- `remote_client_bridge/src/lib.rs`: `9bd2f1f84f824400eca3be1fbd7155cdb09ec484b14f4919d72b0584a1574677`
- `remote_client_bridge/src/pairing_export.rs`: `95ea774b0bac20b8b5fa7934d1c737f279a6d8a1ed7faf8c53b718fc4f84e64b`
- `remote_client_bridge/src/startup_diagnostics.rs`: `a90a175565b6f51484d49b6594dd399f0bbaeb76ffd18f80450b259f217f0192`
