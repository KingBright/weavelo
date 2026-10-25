# Studio / HO5 discovery readback and UDP diagnostics

The signed Studio migration remains completed. It was not replayed, rolled back,
rebuilt or reinstalled during this investigation. No invitation, member key body,
token, Pairing screenshot, new credential, group change or NAS business write was
used. The old clients and verified persistent rollback App remain intact.

## Actual device state

The authenticated Octendra inventory had Studio and HO5 online. Cube's Code
Gateway was offline (last seen `1792900549`); this is **not proof that its Weavelo
process is stopped**. No Windows command or alternative route was attempted.
Book/phone remained offline and were not contacted.

| Observation | Studio Desktop | HO5 Desktop |
| --- | --- | --- |
| Actual executable source | `d722c5a9a2a974e1ceb0ab7e0712f699fe11e021` | `26a9c0c21d9e74fa293f07c54e5a70ebbabdce37` |
| Executable SHA-256 | `301f80c39e9b8077194f037937c814861a474e794f3cc7e05707f84eaad791b2` | `aded05f172f8caf87f60ee9d8cbc9cc5ae3b73521f5cff5c8778fec5f32b5d81` |
| Background / GUI PID | `54713` / `54719` | `144473` / `144550` |
| Product namespace | `fun.hackerlife.weavelo.desktop` | `fun.hackerlife.weavelo.desktop` |
| Display name | `MacStudio.local` | `bazzite` (inventory label is HO5) |
| Device identity digest | `3cacc365…` | `95ab9239…` |
| Saved network-name digest | `879e2e8a…` | `b4a8b395…` |
| Control capability digest | `88ff8fdd…` | `8899327b…` |
| Discovery capability digest | `ec219080…` | `f6004217…` |
| Owner versus GUI group observations | equal | equal |
| Restart required | false | false |
| Control / discovery relay | both connected/active, HTTP 101 | both connected/active, HTTP 101 |
| Current online same-group peers | 0 | 0 |

Both the saved network-name fingerprint and the **actual runtime tunnel group
fingerprints differ**; the device identity fingerprints also differ. This is
not a duplicate device ID or merely the inventory label versus display name.
The current observations therefore do not establish a same-group discovery
failure. They establish two running Desktop owners in different groups. No
filter was loosened and neither node was joined/moved to another group for a test.

HO5's owner and GUI are authenticated to the same installed executable. Its
retained client is also still active, separately at
`~/.local/opt/weavelo/weavelo`, PID `1397692`, source `d5ecef1a…`, SHA-256
`410fa94b…`; its control/discovery digests are a third, distinct pair
(`0629ae58…` / `174a5044…`). It was not changed or used to impersonate a Desktop
acceptance test.

HO5's initial Desktop readback had 59,695 successful UDP sends, zero send errors,
and 1,269 foreign-network rejections; its relay had zero accepted advertisements
from a peer. Studio's relay had accepted 41 advertisements earlier, but its
current live snapshot is empty. Its GUI retains one offline record. A native
`--file-peers-json` query identified that record as `MBPM2MAX.local`, identity
digest `d345998b…`, online=false, rather than HO5. No offline Book connection was
opened. The cached offline row is not evidence of a currently connected peer.

Connected WebSocket transport, increasing `registrations_sent` and received
pongs demonstrate transport activity and registration **writes**, not a server
membership ACK. The native pairing report does not claim registration verified.
No NAS registration database was read or modified by this task; in particular,
the coordinator's unknown `r1064` was not queried/replayed.

## UDP error evidence and narrow correction

Studio's installed binary still reports exactly one UDP send error while its
successful send/self-receive counters continue increasing. The installed
`DiscoverySnapshot` stores only `last_error=send`, with no errno or target class.
The verified background PID's stdout/stderr descriptors both point to
`/dev/null`. A bounded unified-log query for that PID and the exact discovery
announcement prefix returned no messages. The specific historical OS failure
**cannot be recovered from these retained observations**. It is not established
as TCC, local-network permission, firewall, DNS or another particular cause.
No security setting or permission was changed to guess at a fix.

Commit `afdd731a44ba6e64ddac4b2309ebd08c0a09f2de` corrects this diagnostic loss:

- Preserve the last send's OS errno, standard `ErrorKind`, UDP/relay classification
  and IPv6/broadcast/loopback booleans in one bounded observation.
- Never retain the error text, destination address/port, discovery payload,
  device identity, network name or relay capability. The error count and detail
  update under the same diagnostics mutex.
- Add an optional/defaulted DTO field so old snapshots remain readable. The
  peer discovery packet, network filters, registration, identity and permissions
  are unchanged.

This source correction is **not deployed**. Studio still runs the verified
`d722c5a9` build and HO5 still runs its verified `26a9c0c` build. No claim is made
that new diagnostics retrospectively explain the historical error.

## Local validation, separately from cross-device acceptance

From primary main, with the existing shared `~/rust-target` cache initially
unheld and 228.79 GiB free:

```sh
python3 -B scripts/test_stream_settings_model.py --mode discovery-observation
TMPDIR=/private/tmp PYTHONDONTWRITEBYTECODE=1 ./scripts/test_release.sh
```

The fixed Rust entry passed **18 / failed 0 / ignored 0 / filtered 400**. It covers
real local loopback peer discovery, same/foreign-network and self filtering,
name updates, distinct relay counters, metadata-only send errors, old snapshot
deserialization, and a controlled native IPv4-socket/IPv6-target error retaining
its actual OS evidence. That controlled error is **not** the historical Studio
error. Guarded validation ended with exit 0, sources unchanged, owned process
group absent and post-observation 0; no cleanup or alternate cache was used.

The fixed release suite passed **total 856 / pass 833 / skip 23 / failure 0 /
error 0**. There is no change to `discovery.rs` or `discovery_status.rs` between
the two installed source commits `26a9c0c` and `d722c5a9`; the new optional error
field is the subsequent source-only change validated here.

These tests do not constitute a same-group cross-device session, first frame,
remote input, audio, file transfer or long-duration acceptance. The usable Linux
node is in a different group and Windows is not currently observable through its
existing management channel. Current scope forbids expanding group access for
acceptance; those end-to-end checks remain unperformed.

## Evidence handles

All following local JSON is credential-free metadata, under
`.rust-storage/validation-logs/`:

- `weavelo-cross-platform-device-availability-20261025.json`: inventory observation
  `1792914855`, Studio/HO5 online, cube/Book/phone management channels offline.
- `weavelo-ho5-desktop-public-summary-20261025.json`: independent current HO5
  readback, original operation `11e60d24-01c0-4b9d-8aa6-f57dbcbf735b`.
- `weavelo-studio-ho5-discovery-comparison-20261025.json`: exact public digests,
  native owner/GUI observations, comparison results, preserved Studio membership
  metadata/plist, validation log digests; SHA-256
  `73e716d56f691974461660bf1d7b5dda118c79dbcf423981e0387f5834d170c0`.
- Earlier bounded HO5 owner/legacy readbacks are retained by original operations
  `64c30492-b418-414a-a6ec-a9ba0511ff91`,
  `0d596b98-fb0b-4b05-ab2b-3e3cc2f40ec8`, and
  `a4a8acb3-eab7-4c4d-9b22-2de4a8f7c0a1`; none was replayed.

Fixed logs: `/tmp/weavelo-discovery-observation-20261025.log`, guarded receipt
`discovery-observation-ikw1nb2h.jsonl`, and
`/tmp/weavelo-discovery-diagnostics-release-20261025.log`.
