# Weavelo Desktop cross-platform checkpoint — 2026-10-26

This is a current-device checkpoint, not a release or physical-media acceptance.
The primary `main` source checkout was `5303099` at the initial diagnosis and
advanced to `dde1ce8` for the Windows mock-test coverage fix. No package,
installed executable, membership, credential, startup policy, or OS permission
was changed. The existing cube Desktop background was started once. Historical
results in `FOUR-PLATFORM-MATRIX.json` must not be counted as current passes.

| Platform / device | Installed build and actual runtime | Device discovery / interoperability | Video, input, audio, native file transfer |
| --- | --- | --- | --- |
| macOS / Studio | Independent signed Desktop installed; actual binary SHA-256 `66ef666ce608ed8af5699e7eac21d2cd7e2e5034d19ba9ade2dc4430ad0fa6b1`; background PID 69042 and GUI PID 69053 observed; both relay channels connected. Current source revision was not independently extracted in this readback. | One listed peer is `MBPM2MAX.local`, not HO5. Current Studio and HO5 runtime group fingerprints differ. Three peer-probe samples returned `other_endpoint_group_mismatch`, not a same-group discovery failure. | Not executed against HO5 on these builds. The listed peer and connected relay do not prove a visible frame or media acceptance. |
| Linux / HO5 | Independent Desktop executable SHA-256 `5aef25ca02fc7664ef6412d7466f3e32395f391f017984854905d961f685285a`, source `81d772c`; background PID 1772859 and GUI PID 1779219 observed; both relay channels connected. Retained older client was left intact. | Zero same-group peers in three samples; probe returned `no_discovered_peer` and `other_endpoint_group_mismatch`. UDP had successful self-receives and zero send errors; relay had zero peer advertisements received. | Not executed against Studio. Prepared 262181-byte non-sensitive synthetic file is a fixture only; `native_transfer_verified=false`. No formal HO5 screen/input observer is available here. |
| Windows / cube | Octendra management channel online; independent Desktop executable SHA-256 `5e83758de69e0a4b3cdb36f245bf0ef245dbbfc36eba9042ee5332f9c3ab3862`, compiled source `b3fd12181c1e8f20b277d7d092e68dd077a94b11`, product version `2.0.0-alpha.9`. The existing independent profile and install journal were present. After finding no Weavelo process, the new Desktop background was started once as PID 21252; the retained client was not started. | Product discovery state not verified. OS readback found PID 21252 at the installed path with `--background-service`, two established TCP connections and UDP ports 39273/38119. These do not establish same-group discovery or a product-level relay ACK. | No Windows GUI or real stream, input, audio or native file-transfer session was opened. Synthetic protocol tests are listed separately below. |
| Android / phone | Management channel offline in the current inventory; running app and installed build not observed. | Not executed. | Not executed. |

MacBook was online in the management inventory but was not used: Studio covers the
current macOS checkpoint, and the user asked us not to pursue Book/phone for the
Desktop pairing issue. The matrix above does not imply either has passed.

## Cube non-interfering follow-up

The cube primary `main` checkout was clean, then fast-forwarded from `d722c5a`
to `00c7fd3` and then `dde1ce8` through its existing Git remote. These source
syncs did not rebuild or replace the verified installed `b3fd121` executable.
Windows `--product-info-json`
readback named the original GPUI default and independent application ID
`fun.hackerlife.weavelo.desktop`; its conservative `separated_lifecycle=false`
metadata remains a distinct release-capability gate, despite the observed
background process. The signed-in console was active, so no GUI window was
started or user input sent.

From the current Windows source with its existing Cargo cache, isolated synthetic
tests passed: **12** real-time input protocol cases; **22** file-transfer protocol
and runtime cases (including UDP round trips, lost ACK/retransmission, scoped
destination numbering and cleanup); and **4** portable HEVC decode/parser cases
with `WEAVELO_FFMPEG_BIN` bound to the already-installed bundle's FFmpeg. The
HEVC fixture test decoded a 128×72 frame; the native Windows video feature was
compiled, but this is not a D3D11 presented-frame or product encoder test.
Two native Windows physical-key mapping tests passed. An initial run of the
injector recording-contract filter selected **0** cases: its module was
incorrectly excluded from the Windows test target. Test-only commit `dde1ce8`
enables the existing recording API on Windows without calling real `SendInput`;
the native cube rerun passed **6/6** deadline, modifier, ownership and cleanup
cases. These tests did not use the live profile, invoke real input, or transfer
a user file. Their durable
Octendra operation IDs are `ea462825-2071-4859-af0b-5be87ad7b2e2`,
`17757901-3671-4b6d-9411-eba0ebbf1937`, and
`1a5e37c7-518e-492b-81b3-7a664c19f86a` respectively. The key mapping and
mock injector operations are `3a0a5c39-795e-4c92-8112-c1fe745d3a22` and
`17005de2-3924-4550-949e-a068850064f1`.

One attempted `--background-status-json` query failed without a product status
response. Automatic approval review then rejected a diagnostic repeat because
`Client::connect` calls `AppPrivateMeshConfigStore::load`, which can perform
transaction recovery before returning a snapshot. That rejected request was
not executed or rerouted. The OS-only process/socket readback above is narrower;
product relay registration, discovery and group membership remain unverified.
The final independent OS readback still found PID 21252, two established TCP
connections and both expected UDP ports; the config and credential files retained
their prelaunch lengths and last-write times. No GUI was launched.

## Why Studio and HO5 do not see each other

Both live runtime owners have internally consistent control/discovery group
fingerprints, but the fingerprints differ between devices. HO5's independent
profile was created at its original install, and its mesh config/credential file
metadata had no subsequent write in this observation. The Linux independent
installer explicitly reports `pairing_imported=false`; `MeshPairingControl::load_or_create`
generates a separate group for a new private profile. Thus different initial
groups are the supported explanation. This evidence does not prove whether any
past invitation attempt failed before saving.

The GUI's QR-image scan and clipboard join feed the same
`join_mesh_group_from_payload` path and guarded `join_from_invite_code` store
operation. The fixed `pairing-join` suite on primary main passed **11/11**, including
QR/manual equivalence, restart persistence, identity/name preservation, stale
runtime reconciliation and ambiguous-commit handling. This is synthetic-path
validation, not proof that HO5 scanned or joined Studio's live invitation.

To establish cross-device acceptance, the target is **HO5's independent
Desktop profile joining Studio's existing independent Desktop group**. The
earlier user approval covers a temporary copy of HO5's original group in its
existing protected credential backend and the subsequent move, but the current
task is limited to planning; it did not execute either operation. First confirm
no active sessions or transfers, stop the HO5 Desktop owners through their
normal lifecycle, create and verify the protected checkpoint, then carry the
invitation solely through a supported private authenticated channel and use the
product join path. The move changes HO5 Desktop's access to its old group,
including any offline members. The retained old client and Studio's group stay
unchanged. A rollback must use the exact protected checkpoint and verified
original profile pins before new activity, preserving HO5's node ID, display
name and profile. Existing source/Linux fixture checks do not establish a live
checkpoint or private end-to-end invitation route; that missing route blocks
execution without broadening credential handling. No invitation was read or
exported, and no group filter was loosened. After a verified move, repeat live
mutual discovery, then first visible frame, input, audio and bidirectional
native file transfer on the actual installed versions; record each separately.

The prior native pairing-status command was denied before execution by automatic
approval review because its output could contain an invitation. Metadata-only
file observations and the separate non-secret peer probe supplied the diagnosis;
no secret body was read or printed. The denied command was not retried through a
different route.
