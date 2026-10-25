# Independent Linux Desktop upgrade and normal exit

The authorized scope is a normal version-bound HO5 Desktop upgrade, preserving
the retained client, original profile/node/group, service policy and a verified
old-app rollback. No real group change or invitation transport is part of these
source/tests. The next necessary package must include d138c7d's lock-lifetime fix.

## Product exit contract

The additive private GUI request `UpgradeQuit` and `--gui-upgrade-quit-json`
require the expected generation and executable SHA256. The status capability is
`gui-upgrade-quit.v1`; an old owner is never presumed to support it. The GUI thread
freezes new work, checks every frontend owner's sessions/pending state and a
shared activity counter. All window adapters, receive leases, directed sending
tasks and accepted file-send futures hold that counter, including time before a
future is first polled or while it waits for the serial sender. Unconfirmed prior
cleanup also refuses. This quits the attached GUI only, leaving its background
independent. Normal user Quit controls keep their previous semantics.

A prepared cleanup reply is written before normal application exit. The CLI
registers an exact native process-exit observer first, validates that reply and
then requires the original PID's exit, no replacement endpoint and a free frontend
lock. A lost reply/timeout is unknown, never authorization to issue another quit.

The additive background capability is `shutdown-idle.v1`. Its upgrade CLI requires
plan generation, executable hash and PID before any stop request; reconnecting to
a replacement of the same executable cannot stop that replacement. The host loop
expires only handshake entries past their existing TTL and atomically refuses
active peers, closing work, authenticated/pending handshakes or directed delivery.
The existing normal shutdown API remains unchanged. Actual cleanup, process exit,
supervisor stopped state and endpoint absence remain separate observations.

## Installer transaction and rollback

`scripts/upgrade_linux_desktop.py` reuses `install_linux_independent.paths`, package
verification and startup payload generation, not the first-install apply or the
retained-client `LinuxUpdate` object. It binds the original Desktop state.json by
hash and keeps it immutable. Unit/launcher/desktop/icon bytes, loaded fixed
ExecStart, NeedDaemonReload=no, supported restart/drop-ins and the original startup
intent must agree. It never enables, reloads or rewrites a service. No Environment
or EnvironmentFiles values are queried.

The fixed original work/upgrade journal and rollback-app exchange slot are private.
App, executable and staging must already share a filesystem and the same deepest mount ID; ambiguous stacks, app/slot subtree mounts and same-device cross-mount exchanges refuse before any stop. Candidate security labels/ACL
metadata must equal the old image before any owner stops; there is no label,
permission or platform-security repair. Native renameat2 exchange retains the exact
old app inode/tree as rollback, with both parents and the app trees synced. This
does not claim hardware power-loss testing. The fixed retained transaction blocks
another upgrade until its ownership/retirement is separately resolved; no unknown
rollback slot is overwritten or archive generation created.

The profile check stats the existing canonical files and locks; it never reads,
hashes, copies or prints mesh.secret. A pending profile recovery blocks the upgrade.
After normal exits, both lifecycle locks and the profile-directory lock must be
available. App/startup/profile tuples are rechecked before/after exchange and after
activation. Actual process images are verified through /proc against the pinned
ELF; a package checksum alone is not runtime verification.

Apply requires the exact plan digest and cannot replay an existing transaction.
Read status after an uncertain stop/exchange/start. Explicit recovery binds the
original transaction ID and permits backward exchange only with a recognized tuple,
unchanged original configuration and stopped owners. Complete matching pending
metadata is recognized by strict schema, fields, phase adjacency and actual app
tuple; partial/unknown records remain untouched. Historical activation receipts
are not treated as current PIDs. A verified rolled-back terminal pending commits
only its metadata, even if the old owner is still running or was normally closed;
it never exchanges or starts again.

## Completed Studio validation

All use primary main and the original shared incremental cache. Fixed entries
reported source unchanged and owned process groups absent:

| Fixed mode | Result | Receipt below .rust-storage/validation-logs |
| --- | --- | --- |
| upgrade-quit | 4 passed | upgrade-quit-n8h8wyiw.jsonl |
| host-upgrade-stop | 1 passed | host-upgrade-stop-ebxsppyc.jsonl |
| upgrade-handshake | 1 passed | upgrade-handshake-pdvzw821.jsonl |
| lifecycle-service-contract | 11 passed | lifecycle-service-contract-vih5hpr0.jsonl |
| native-file-cli | 8 passed | native-file-cli-344l2pkm.jsonl |

The initial file-command run had 7 pass/1 failure because its disposable temp root
was not private. Only that owned fixture now explicitly creates a 0700 directory;
the product check is unchanged. The first upgrade-quit compile found an incorrect
runtime method receiver; it was corrected before the successful fixed run.

Final `./scripts/test_release.sh`: **903 total, 879 passed, 24 conditional skips**,
zero failures (`/tmp/weavelo-linux-upgrade-release-mount-id.log`). This includes
32 isolated upgrade fixtures: every recorded phase, timeout/unknown refusal,
exchange acknowledgement loss, complete/partial metadata pending, unknown fields,
terminal recovery without activation replay, startup drift, concurrent refusal and
security-label/mount-boundary drift. macOS mocks only the Linux xattr query for these storage
fixtures; actual Linux xattrs/renameat2/systemd and the real GUI remain separate.
Independent source review, including the final label-equivalence, same-device
bind-mount gate and fixture-only permission change, found no remaining concrete
blocker. No real owner was stopped by these tests.

## Live preflight boundary

HO5 still runs source 26a9c0c, background 144473 / GUI 144550 at the last readback,
with no GUI sessions, pending requests or receive lease. Its old GUI has no Quit
IPC and its service lacks idle-only shutdown. The source exposes normal Quit GUI,
attached-window close and Quit Weavelo (the latter normally exits both owners),
but there is no verified formal GUI route to operate that old window. A future
binary cannot add a protocol to those current processes. The plan therefore
reports unsupported and refuses before creating an upgrade transaction or stopping
anything. Do not substitute signals, change registration, enable ComputerUse or
reinterpret Stop (which only ends a receive lease).

The authorized read-only loaded ExecStart response was the standard 10-field
a(sasbttttuii), canonical Desktop executable plus --background-service, with
NeedDaemonReload=no. App parent/app/executable/installer state had the same SELinux
metadata digest b1efb479e94224168121b808e28a580132725e0102b769d676a4803eca493ac3.
An initial combined query including environment properties was rejected by automatic
review for possible sensitive disclosure; only non-secret execution metadata was
subsequently queried. No environment values were read and no policy was changed.

The Studio executor briefly disconnected. After reattachment, readback confirmed
the last source/check batch had actually completed (29 fixtures passed), so it was
not replayed. No install, build, push or recovery had an unknown live outcome.

Native HO5 tests, the next formal package and its read-only upgrade plan remain to
be recorded after the source commit. Actual GUI exit/install/media acceptance is
not established by these fixtures. The existing transport still lacks a verified
private cross-device invitation stream; real checkpoint creation and group move
remain paused without using terminal_input or ordinary files for secrets.
