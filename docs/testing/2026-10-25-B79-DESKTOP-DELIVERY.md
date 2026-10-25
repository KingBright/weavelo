# b79e3b9 Desktop delivery

The user authorized delivery of the reviewed profile transaction and diagnostics to
Studio and available HO5, preserving identity, configuration and rollback. This
record distinguishes compiler output, formal packages, installation and live
pairing. No real invitation was exported during the preflight/build/package work.

## Reused native build and package entries

Both existing primary main checkouts were clean at b79e3b9. HO5 was fast-forwarded
through the existing origin, preserving its original source/cache contract. Both
native builds used build_identity_gate.py build, the existing incremental cache,
fixed producer lock, fixed features and the source snapshot
25fe1caa5264f7cf91f4d72fddf3d1408718332dd4688e96262e2ba6be101abd.
Only inherited RUST_LOG was removed in build children, as the existing contract
requires. No compiler/identity override, new target, certificate or credential was
introduced. Each platform was built and packaged once.

| Evidence | macOS arm64 | Linux x64 (HO5) |
| --- | --- | --- |
| Compiled source | b79e3b9ea27f10c0385b068e13cab722d3ecb30b | same |
| Build identity SHA256 | 4ce5dc6a589d0bc98eb2607901dd1180a615d19f6eb83976fca927981a78ba70 | bc94c751f7b8b81eed7f74336ce3f3d3619072bf4c2e980591c5e267d2f55edb |
| Observed build receipt SHA256 | 567e6556b65325665699fd414cd5afb676f227d283c52631ccf0e92ff7f929a9 | aced89516c1d68abe412cef7b428445e7902d48b5256737de1b1b2714f10f803 |
| Compiler binary SHA256 | f58b2fcb4ba911b1268356f8d750f58b76ba216c4632b5ec5333da2fabe306eb | 802fea94b091f199648f4c152b57d9d281b39ec78a93a9b157c3e91d7012d3cc |
| Signed executable SHA256 | 66ef666ce608ed8af5699e7eac21d2cd7e2e5034d19ba9ade2dc4430ad0fa6b1 | unsigned native Linux artifact |
| Archive SHA256 | 028072ef9a1ca5b6ff46f3125e6e66ee2dc9280d40219d083788ed8d5a93fe54 | a8e7561e2aff4a21bef6cf837fa96b0f1cc7e4d6b6293612df55d151540b1af5 |
| Manifest SHA256 | e2e2f5dace2ee889a4d05250d658aa872abd338775bf724eef2d790be93fa31e | b1146d03f0283cad75e1f7e123294a246bedf143ac7e4b8ab4fc5adfa5c56802 |

macOS uses package_macos_unified.sh --desktop, new build 20261025.10 and the
existing pinned d38bedce6830fb01240f97c11c7984514ab715c0 leaf/designated
requirement. Linux uses package_linux_desktop.sh and its original native ELF
receipt. Both current-output transactions committed and completed cleanup; both
archives were independently verified. No NAS upload occurred. The normal default
package paths remain unchanged; an archive checksum alone is not authorization or
physical acceptance.

HO5 operations: build a0346a94-87da-4258-b20a-60d17e784cf6, independent compiler
readback 6a70b65d-e515-4d02-8bbb-17f5fc3482bf, package
4a2228e1-0bcf-4c4b-956a-1d9c7f1d09e1, independent package readback
dd52f040-b112-4074-a9e4-98d35f4abd10. Native build/package exit 0 is separate
from installation, GUI rendering, discovery or media acceptance.

## Normal macOS upgrade rollback retention

Preflight found the old normal upgrader deleted its old application staging tree
after success. The retained signer-transition rollback is pinned to the earlier
migration tuple and must not be reinterpreted for this update. Before applying,
the normal upgrader now prepares one fixed private .desktop-upgrade.rollback slot
with the verified previous app, exact install registry bytes and launch plist.
It never overwrites a different slot and does not modify the signer-transition
rollback, retained clients, profile or signing policy. This is installed-app
rollback state, not another build cache/output generation.

The explicit desktop_upgrade_rollback.py default is a read-only plan; --apply
requires that plan digest. It checks both same-signer app trees, source/registry
reconstruction, launch policy and the current runtime state. Before exchange it
rechecks slot/app/registry/plist/pending-profile state under the original installer
and instance locks plus the new core's profile-directory lock. Registry bytes are
restored exactly, and final app/registry/plist/membership are revalidated. Runtime
restoration preserves the shape observed when rollback is requested, including a
closed GUI; it does not promise to reopen the upgrade-time GUI.

A rollback journal already present blocks replay. Interrupted rollback remains
preserved for inspection of restore.json and actual tuples; this entry does not
claim automatic interrupted rollback recovery or hardware power-loss acceptance.
A pending profile transaction must finish in the new core before rolling back to
an older binary. No user profile credentials are read/copied into the app rollback.

Fixed regression: 859 total, 836 passed, 23 conditional skips, zero failures.
Evidence: /tmp/weavelo-upgrade-retention-reviewed.log. Includes real macOS atomic
exchange on disposable app fixtures, exact registry/profile preservation and
post-quit slot/registry/plist/pending conflicts refusing before exchange.
Independent read-only review found no remaining concrete blocker for same-signer
upgrade and uninterrupted explicit rollback. The compiler/package receipts above
remain bound to b79e3b9; installer-only changes do not justify a duplicate product
build or mutation of an already numbered package.

## Live gates not yet claimed

Studio's original Desktop owner was authenticated with zero GUI sessions/pending
requests and no receive lease; prior app/build was 20261025.9. Installation and
post-install readback will be recorded separately after the authorized apply.

HO5's first-install journal is installed-not-started, with a real running owner
from 26a9c0c. The existing independent installer is first-install-only. The retained
client LinuxUpdate adapter explicitly rejects separated lifecycle and targets a
different app/profile/service. It cannot be used against Desktop by changing
paths. A scoped independent Desktop upgrade adapter must preserve the original
journal/profile/service and retain its rollback before applying; no raw binary
replacement or old-client update is acceptable.

The existing dedicated product export requires a separately inherited anonymous
pipe (owned FIFO, no links, fd > 2 and not an alias of stdout/stderr). The join entry
uses private stdin and a credential-free report. Code Gateway terminal_exec has
stdin EOF; terminal_input forbids credentials. File upload/sync is not that stream.
HO5 SSH service/public identity was observed via authenticated management, but
Studio's existing SSH agent has no identities and no matching known-host route was
found. No SSH trust, keys, passwords or new transport permissions were added. No
real secret was read or printed to prove a route.

An original Desktop group checkpoint still needs a supported protected-backend
interface and recovery fixtures; the transient profile transaction and application
rollback are not permanent original-group recovery sources. No real group move or
video/input/audio acceptance is implied by these build/package/fixture results.

## Studio compatible install completed

The single formal install_macos_desktop.py --upgrade --apply completed. Actual
installed build is 20261025.10 with the same pinned d38bedce signer and bundle ID.
The running background PID 69042 and frontend PID 69053 independently reported
b79e3b9; executable SHA256 is
66ef666ce608ed8af5699e7eac21d2cd7e2e5034d19ba9ade2dc4430ad0fa6b1.
Install registry SHA256 is
30eaf2d9dd3956daa05912ca37369e35a94685327f5b7ea2ca0eb1a3a8723c52.
Public device/group/name stayed equal to the preflight, and launch plist SHA256
remains 113253c0990cdc224f1a25c9d9025bb0731dd6aabcbfe4dedb0abb509cc7ea54.
The new same-signer old-app rollback plan verified independently; the prior
signer-transition rollback and retained legacy application remain present.

Formal Computer Use read the actual new Weavelo window: original dark screen-first
layout, compact top controls and Device & Network Control drawer. Devices is
selected, This device is MacStudio.local, filter All, no devices in this filter,
and the center reads Ready for incoming and outgoing sessions. No pairing page
was opened or invitation captured. This verifies the idle rendered window only,
not connected video/input/audio. Native install receipt remains at
.rust-storage/validation-logs/b79e3b9-studio-compatible-upgrade.json.
