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

## First native HO5 readback and release-read correction

HO5 cleanly fast-forwarded to b02b26f. Its fixed release script completed **903
total: 861 passed, 41 skipped, 1 error**, operation
bb812f53-4abd-45dc-b945-bec3784dccc1, log
`/tmp/weavelo-ho5-b02b26f-python.log`. The error was the existing Android publication
fixture's first read: release_artifact_lifecycle.hashed compared the entire stat
result, including access time. A Linux read can update that field without changing
the release file. No Android package or real output was published by the fixture.

The project-local adapter's hash/receipt readers now hold a fixed no-follow file
descriptor and compare descriptor plus canonical-path identities: device, inode,
UID/GID, mode, link count, size, mtime_ns and ctime_ns. Only atime is excluded. The
receipt bound remains 256 KiB. Content/protection/inode/path changes still refuse;
there is no change to the public storage authority, signing or producer gates.
Three added fixtures cover first reads with old atime, exactly which fields remain
protected, and content/mode/inode/symlink mutation during a read. Independent review
found no concrete blocker. Studio fixed regression then passed **906 total, 882
passed, 24 skipped**, zero failures (`/tmp/weavelo-release-stable-read-regression.log`).
The native Linux repeat must pass before a formal candidate build starts.

That native repeat on 02fa376 completed **906 total, 865 passed, 41 skipped**,
zero failures (operation ff3e10c6-05b2-4f7c-87e5-7185696aaaa5). The subsequent SDK
pins, mount and profile checks succeeded, but the old global same-UID /proc scan
failed on unrelated protected PID 1865/exe (EACCES). No process was stopped or
permissions changed; the denied executable metadata was not fetched another way.

Owner discovery now queries only the fixed service MainPID and, while its role
lock is busy, the canonical profile's private GUI endpoint. Each resulting PID
still requires native loaded-image/inode/hash verification. GUI PID comes from
NetworkDiagnostics, matched to Status generation/hash and the original renderer;
it is not guessed from a generation string. MainPID zero requires inactive plus
all three locks free. Busy frontend with missing/conflicting private state or any
known target PID denial still refuses. Startup performs one start and polls only
observations through transient process readiness. The proof is for registered
owners of this canonical profile, not all same-named processes on the machine.

Six new fixtures exercise that scope, busy locks, target EACCES, GUI/supervisor
replacement and non-replayed activation. Independent review found no P1/P2 in
this increment. Studio fixed regression: **912 total, 888 passed, 24 skipped**,
zero failures (`/tmp/weavelo-linux-profile-owner-release.log`); **38** isolated
upgrade fixtures passed. Native Linux repetition and the formal package remain
required before any real upgrade.

## Native registered-owner and systemd compatibility readback

HO5 on 59228dd passed **912 total, 871 passed, 41 skipped** (operation
81fecf53-c38d-4f7b-94ea-10aa59c146d0). Its five fixed Rust modes also completed
exit 0, operation add411af-6798-453e-a506-a13e3f237653: upgrade-quit,
host-upgrade-stop, upgrade-handshake, lifecycle-service-contract and native-file-cli.
No formal candidate build or install had started at that point.

The next read-only service check found that this systemctl omits five deprecated
Exec properties even with --all. All other requested fields, supported restart,
NeedDaemonReload=no and canonical FragmentPath matched. Explicit standard D-Bus
reads then proved ExecStartPre/Post, ExecCondition and ExecStop/Post each had the
fixed a(sasbttttuii) type with zero entries (operation
f81ca718-f903-4617-84be-62c12fca32f6). Values/arguments were not disclosed. Actual
unit KillMode is control-group and KillSignal is 15; neither was changed.

Python now normalizes only missing members of that exact ExecHook set after a
typed-empty D-Bus proof. Other missing fields, unknown types and nonempty commands
still reject. The metadata getter has a fixed execution-property allowlist and
refuses Environment/EnvironmentFiles requests. The new binary's Linux stop-policy
reader applies the same explicit proof for ExecStopPost before the existing PID,
restart, active-state and activation checks. This fixes an existing client-side
compatibility rejection; it does not grant the already running old binary a new
exit capability or replace it through signals.

Independent source review found no P1/P2 in this increment. Studio fixed lifecycle
mode passed **12**, zero failures, source unchanged/group absent
(`/tmp/weavelo-systemd-typed-stop-rust.log`). Fixed release regression passed
**915 total, 891 passed, 24 skipped**, zero failures
(`/tmp/weavelo-systemd-typed-property-release.log`), including **41** isolated
upgrade fixtures. Native readback/build/package and actual installation remain
separate gates.

## Native candidate 81d772c and remaining admission gates

HO5 repeated the fixed Python release regression on 7fd92f6: **915 total,
874 passed, 41 skipped**, zero failures (operation
ffcc014b-29db-4f8d-aa06-66f85fd14a53). Its native lifecycle compilation exposed
E0507 in the Linux-only service policy branch: moving linux_service out of the
static identity reference. Commit 81d772c borrows that field instead; independent
source review found no remaining issue in that correction. The native fixed
lifecycle mode then passed **12**, zero failures, with sources unchanged and its
owned process group absent.

The single formal native identity build used primary clean main 81d772c and the
existing target/verified SDK; operation 74111879-ba5b-4903-993b-a305b6d5dfda
completed exit 0. Build started 2026-10-25T12:53:06.675336Z (PID 1675513).
This candidate includes the profile transaction and d138c7d lock-lifetime fix.

| Binding | SHA-256 |
| --- | --- |
| Source snapshot | 7247a0d596cd9ff702f6e8e837e799ee783aaf1fce427cc3f5771fb8c5147950 |
| Compiled identity | abb1efd97378fa80694e5f95ba5b9f71ff49b3dafe2eca6d0d4d208defd8b3f5 |
| Compiler ELF | 5aef25ca02fc7664ef6412d7466f3e32395f391f017984854905d961f685285a |
| Observed build receipt | 37479182ce03436ccf92db3037b5f295808e12fd3c8d90fe1aa70170fc6f4547 |

The receipt is `.rust-storage/validation-logs/81d772c-linux-release.observed-build.json`
on HO5. It executes the exact ELF's non-starting product-info entry and confirms
restored-original-gpui, original GUI/native video compiled and Desktop identity.
This is compilation/provenance evidence, not installed or physical media evidence.

Formal packaging used the unchanged package_linux_desktop.sh wrapper through bash
(the tracked wrapper has no executable bit; HO5's copy is 0600). Initial local
orchestration path/mode checks failed before starting the packager and were read
back, not treated as successful packaging. The actual packager started once at
12:57:42.782867Z, PID 1682109, operation
1e6feaa1-6c22-4d31-b587-3149f19690f3, and exited 1 at public release-admit:
`Linux release observation unknown: process-command-identity-unconfirmed`.
No alternate publisher, raw ZIP, observer exemption, process termination or
permission change was used. The public helper remains outside this project's
implementation ownership.

Independent readback 2706a512-b307-4d8e-b92f-937af5b1a0c9 re-executed only the
new ELF's product-info flag, verified the retained b79e3b9 ZIP and manifest, and
confirmed its original SHA-256 a8e7561e2aff4a21bef6cf837fa96b0f1cc7e4d6b6293612df55d151540b1af5.
The producer lock is free and the packaging auxiliary stage is absent. The HO5
public helper does not advertise the newer --observe-only preflight option;
no mutating preflight or manual registry cleanup was substituted for it.

Two additional real installation gates remain. Existing private profile.conf and
relay.conf drop-ins use Environment directives. Only paths, ownership/mode and
directive names were observed; no values were read, hashed or reported. Merely
preserving their stat tuple cannot establish that the next service activation
uses the current runtime profile: a previously reloaded configuration can differ
from its still-running process. Independent review therefore keeps activation
blocked rather than weakening the gate. The running old GUI also lacks the new
upgrade-quit capability, and the old background advertises shutdown.v1 but not
shutdown-idle.v1; neither can be retrofitted by the candidate.

Independent installed-state readback (operation
0bffb67d-dfa4-4ab2-a526-ce34e1283c91) still found source
26a9c0c21d9e74fa293f07c54e5a70ebbabdce37, ELF
aded05f172f8caf87f60ee9d8cbc9cc5ae3b73521f5cff5c8778fec5f32b5d81,
background PID 144473, GUI ok with zero pending/sessions and no lease. No upgrade
transaction exists. No app exit, install, real group move or credential export
was performed. Confidential invitation transport and protected original-group
recovery remain separate required gates; mutual discovery, displayed video,
input and audio remain physically unaccepted.

## Formal package and reviewed old-version bootstrap continuation

The release observer failure was not reproduced by one initial and three bounded
fresh repetitions of its original before/arguments/after process contract. All
four had complete command identity coverage; arguments were not disclosed.
Read-only authority metadata then confirmed current_update absent, the old ZIP
unchanged and all exact owned stage paths absent. With this new evidence, the
same formal packager was attempted once, without modifying the observer or ELF.
Operation 5e1ecdf7-3c2c-49f1-91cc-623ff01ddc53 completed exit 0,
current_committed=true and cleanup_status=completed. Source remains 81d772c;
packaging-source 0874351 differs only by the preceding evidence document.

| Formal current Linux artifact | Binding |
| --- | --- |
| ZIP SHA-256 | 94bb46ede676ee94ad0e872064256f6ce754084d4cba1aed89f17918ebf22df2 |
| ZIP bytes | 18316751 |
| Manifest SHA-256 | c1f159415c44caa5b305cef5cfe9e198bbff197ad1f8c76578f2961eb6b4c334 |
| Package receipt SHA-256 | d53240426a61064dfb1ba3086538ddb8fe1f38f7553b3c18010447d116301771 |

The newly authorized bootstrap uses the old release's existing normal lifecycle,
not an invented capability: its attached primary-window close handler releases
input/closes sessions and calls cx.quit; its background awaits ctrl_c/SIGINT and
shutdown_host. No SIGTERM handler was found. Source- and SHA-bound
quit_linux_desktop_legacy.py obtains both native pidfds before requesting a single
normal KWin close, then one SIGINT through the exact original background pidfd.
Visible active/pending GUI work or a receive lease still refuses. Old releases
cannot atomically freeze every new activity: receipts explicitly say this is
normal quit, **not idle-only**. Timeouts/unknowns persist the intent and cannot
replay. The ordinary upgrader remains unchanged in requiring the original owners
stopped or supporting its normal upgrade API. In this bootstrap it runs only
after those actual old process exits and all three stopped locks are confirmed.

HO5 uses a Wayland window, so X11's client list did not identify the target. The
existing KWin public scripting API exposed exactly one normal, closeable,
non-dialog Weavelo window for native verified GUI PID 144550/start tick 117542761,
UUID {3ce50827-6483-47a7-a87b-c870902a5d43}, matching Desktop application ID.
The metadata-only temporary probe was unloaded; no window action occurred.
Background PID/start tick were 144473/117542742. These are observations, never
permanent PID constants: actual quit must recreate and compare its whole plan.
KWin's documented closeWindow method is a normal compositor close request, not
killWindow or injected desktop input ([KDE API](https://develop.kde.org/docs/plasma/kwin/api/)).

An attempted complete /proc/PID/environ read was rejected by automatic review
because filtering afterward still probes potentially private credentials. It did
not execute, and was not retried through another process-environment reader.
Instead linux_desktop_profile_launch.py adds one private, exclusively created
drop-in with an exact /usr/bin/env public-profile assignment and direct product
exec. Original unit/profile/relay files and their values remain untouched. The
canonical value is explicitly applied after systemd environment processing, so
no secret environment read or condition-to-exec race is needed. Native transient
systemd fixture 78e8b1aa-aeba-4bab-b4da-e5707b744bab passed even with a synthetic
wrong inherited value and UnsetEnvironment; the actual product unit was unchanged.

The startup correction has its own durable create/reload record and explicit
stopped-state backward removal, never auto-activation. Default authorization only
accepts completed with no pending metadata. Incomplete/unknown files remain
protected; complete matching terminal removal pending commits metadata only.
Recorded inode/stat identity is rechecked immediately before removal. Application
rollback retains this canonical startup correction; removing the correction is a
separate stopped-state action, not a claim of fully restoring the original startup
configuration while running. Original environment files are stat-pinned, never
read/hashed/copied. Whole configuration CAS is part of both normal-exit boundaries.

Independent source review found the four initial concrete issues fixed. Their
new fixtures cover incomplete authorization, terminal recovery without reload,
same-content inode replacement, and configuration drift before close/interrupt.
The first fixed Studio regression exposed seven existing Windows-fixture errors
because its temporary home retained macOS's /var alias; only the disposable home
now resolves canonically, preserving the production linked-path rejection.
Final native repetition, startup correction, normal exit, installation and actual
media remain separate steps; no physical result is inferred from these fixtures.

Reviewed Studio `./scripts/test_release.sh` completed **944 total, 919 passed,
25 skipped**, zero failures (`/tmp/weavelo-linux-bootstrap-reviewed-release.log`).
The focused startup set passed 18; the normal-exit set had 10 passed/1 native-Linux
skip. Native Linux must execute that owned-child pidfd/SIGINT test before applying.
