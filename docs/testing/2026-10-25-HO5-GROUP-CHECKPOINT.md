# HO5 original-group checkpoint gate

The user authorized a temporary recovery copy in the existing protected credential
backend and, after verification, moving HO5 into Studio's current Desktop group.
This source phase does not perform that move or export an invitation.

`scripts/mesh_profile_checkpoint.py` is a Linux-only adapter for the existing
app-private file credential backend. It adds no encryption or OS access rights.
The profile must be canonical, private and owned by the ordinary user; nontrivial
POSIX ACLs refuse, and SELinux labels must match the existing credential file
before payload write and after publication. Both background and GUI lifecycle
owners must be stopped. The existing core profile-directory lock is retained.

Create binds public group/node digests and keeps one exclusive private checkpoint
inside that backend. It checks the existing core recovery size budget before
copying. Restore stages the unchanged core v1 backward-recovery record after
checking the checkpoint ID, current public pins, node and display name. Normal
core startup verifies and consumes that record; the helper does not rewrite the
canonical pair itself. Unknown pending files are preserved and block replay.
Reports contain no credential, invitation, or credential-derived public verifier.

Synthetic fixtures cover each write/fsync/rename boundary, live-owner/concurrent
refusal, unknown pending state, restart recovery, protection/label refusal,
redacted CLI errors and exact Rust whitespace/metadata compatibility. The Rust
fixture calls the helper and then the real core recovery implementation. macOS
fixtures mock only the Linux ACL query and are wire/storage tests; the real CLI
refuses macOS. They do not certify a macOS protection adapter.

Studio fixed release regression: **871 total, 847 passed, 24 conditional skips**,
zero failures (`/tmp/weavelo-profile-checkpoint-source-validation.log`). Native
fixed mesh-profile regression: **38 passed, 0 failed, 0 ignored**
(`/tmp/weavelo-profile-checkpoint-core-final.log`, receipt
`.rust-storage/validation-logs/mesh-profile-rc0wfhia.jsonl`). Independent read-only
review found no remaining specific source blocker, including the final recovery
budget and private-file-before-label checks. Actual Linux isolated protection
fixtures and a real original-group checkpoint remain separate gates.

Studio's installed 20261025.10 remains the verified b79e3b9 executable; these helper
and test additions do not mutate that numbered package or justify rebuilding it.
HO5's b79e3b9 package is verified, but its running app remains the prior 26a9c0c
version. Its first-install-only entry cannot upgrade an existing Desktop app, and
the legacy updater explicitly targets a different profile/lifecycle. A scoped
Desktop upgrade adapter and a verified normal GUI close route remain necessary;
no raw replacement, service stop or forced GUI termination was performed.

The LAN SSH synthetic pipe probe exited 255 with connection timeout. The overlay
probe's original terminal is no longer retained locally, so its result is not
claimed and it has not been repeated. No real invitation was read to test a route.
Current Code Gateway terminal input forbids credentials, and file upload is not
an anonymous private byte stream. The product's existing export-pipe/private-stdin
entries still need a reachable authenticated SSH stream or a dedicated nonlogging
opaque stream attachment. No new listener, key, trust or plaintext fallback was
introduced. Actual checkpoint creation, group change, mutual discovery and
connected video/input/audio acceptance remain unperformed.

## Native Linux fixture readback and profile-lock release

HO5 fast-forwarded its clean primary main to 261b851. Its fixed Python regression
completed: **871 total, 830 passed, 41 conditional skips**, zero failures; operation
e513055a-cf4f-42ea-a5c4-d5f558219176. These include real Linux protection queries
on disposable synthetic profiles. No real profile was opened by this helper.

The first native mesh-profile run exited 101: **35 passed, 3 failed** with profile
busy at sequential re-locks; the new helper-to-core recovery fixture passed.
Receipt: `.rust-storage/validation-logs/mesh-profile-xxnuy26i.jsonl`, operation
73204df9-3d48-44b2-99da-81265cb6c8b2. No process remained. A separate synthetic
Linux descriptor probe (82661300-11b6-44e9-8b72-fcf8003c3108, exit 0) confirmed
that a duplicated open-file-description retains flock after the owner's fd closes,
and explicit unlock releases it while that duplicate remains open. Concurrent
fork/exec in the new parallel fixture is a mechanism consistent with the failures;
individual failed fork descriptor lifetimes were not traced.

The core now uses a scoped lock guard: explicit unlock before close on normal,
error and unwind paths. Successful acquisition is wrapped before later validation.
No waiting, retry, lock bypass or test serialization was introduced. A deterministic
dup regression verifies live-owner rejection, release with an inherited description
still open, and that closing that old description cannot release a later owner's
lock. Independent source review found no concrete blocker.

Studio's final default-parallel fixed mesh-profile run passed **39 tests**, zero
failures/ignored; sources unchanged and owned process group absent (receipt
`.rust-storage/validation-logs/mesh-profile-fb2r0d8x.jsonl`). Fixed Python regression
passed **871 total, 847 passed, 24 skips** in the ordinary local-user environment
(`/tmp/weavelo-profile-lock-scope-release-native.log`). An earlier sandboxed run
failed because HOME temporary fixtures and /bin/ps were denied; the fixed script
and safety/acceptance thresholds were not changed. Linux repeat remains a separate
readback after source synchronization. The installed/package b79e3b9 binaries do
not contain this subsequent production lock-lifetime fix.
