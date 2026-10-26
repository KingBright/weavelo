# Weavelo Desktop cross-platform checkpoint — 2026-10-26

This is a current-device checkpoint, not a release or physical-media acceptance.
The primary `main` source checkout was `5303099` when inspected. No package,
installed client, membership, credential, service, or OS permission was changed
for this checkpoint. Historical results in `FOUR-PLATFORM-MATRIX.json` must not
be counted as current Desktop passes.

| Platform / device | Installed build and actual runtime | Device discovery / interoperability | Video, input, audio, native file transfer |
| --- | --- | --- | --- |
| macOS / Studio | Independent signed Desktop installed; actual binary SHA-256 `66ef666ce608ed8af5699e7eac21d2cd7e2e5034d19ba9ade2dc4430ad0fa6b1`; background PID 69042 and GUI PID 69053 observed; both relay channels connected. Current source revision was not independently extracted in this readback. | One listed peer is `MBPM2MAX.local`, not HO5. Current Studio and HO5 runtime group fingerprints differ. Three peer-probe samples returned `other_endpoint_group_mismatch`, not a same-group discovery failure. | Not executed against HO5 on these builds. The listed peer and connected relay do not prove a visible frame or media acceptance. |
| Linux / HO5 | Independent Desktop executable SHA-256 `5aef25ca02fc7664ef6412d7466f3e32395f391f017984854905d961f685285a`, source `81d772c`; background PID 1772859 and GUI PID 1779219 observed; both relay channels connected. Retained older client was left intact. | Zero same-group peers in three samples; probe returned `no_discovered_peer` and `other_endpoint_group_mismatch`. UDP had successful self-receives and zero send errors; relay had zero peer advertisements received. | Not executed against Studio. Prepared 262181-byte non-sensitive synthetic file is a fixture only; `native_transfer_verified=false`. No formal HO5 screen/input observer is available here. |
| Windows / cube | Octendra management channel online; independent executable exists at the established Desktop path with SHA-256 `5e83758de69e0a4b3cdb36f245bf0ef245dbbfc36eba9042ee5332f9c3ab3862`, product version `2.0.0-alpha.9`. Two read-only process queries found no running `weavelo.exe`. The retained installation is separate. | Not executed: installed bytes and an online management channel do not establish a running Weavelo owner or group membership. | Not executed. No GUI, stream, input, audio or file-transfer result was inferred from installation. |
| Android / phone | Management channel offline in the current inventory; running app and installed build not observed. | Not executed. | Not executed. |

MacBook was online in the management inventory but was not used: Studio covers the
current macOS checkpoint, and the user asked us not to pursue Book/phone for the
Desktop pairing issue. The matrix above does not imply either has passed.

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

To establish cross-device acceptance, HO5's independent Desktop would need a
controlled move from its existing group to Studio's group through the supported
private invitation transport, after a protected checkpoint of the original group
and confirmation of no active session or transfer. This changes HO5's access to
its old group. Existing source and Linux fixture protection checks do not prove a
live checkpoint or private end-to-end invitation route. This checkpoint did not
read/export an invitation, perform the move, alter the retained client, or loosen
discovery filters. After a verified move, repeat live mutual discovery, then
first visible frame, input, audio and bidirectional native file transfer on the
actual installed versions; record those independently.

The prior native pairing-status command was denied before execution by automatic
approval review because its output could contain an invitation. Metadata-only
file observations and the separate non-secret peer probe supplied the diagnosis;
no secret body was read or printed. The denied command was not retried through a
different route.
