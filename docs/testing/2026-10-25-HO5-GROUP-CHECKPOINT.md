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
