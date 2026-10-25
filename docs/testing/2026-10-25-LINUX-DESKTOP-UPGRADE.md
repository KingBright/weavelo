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

## Actual startup correction, old exit and strict stock-unit retention

HO5 native fixed regression on 643fd6b completed **944 total, 903 passed,
41 skipped**, zero failures, operation 8cdd9222-a59a-46c5-adb0-a426011a938d.
The owned native pidfd/SIGINT child fixture actually passed. This was not repeated
after the already completed run.

The bound public-profile correction plan SHA-256 was
36aad32a41242b93137bc98befe43e4d1dd508795a79bac08602ab843cd51e49.
A capacity/connection interruption occurred before apply; readback first proved
no record, pending, owned drop-in or app transaction existed. The subsequent
single apply operation ba114a2e-a7b1-4421-8627-8512167a2f32 started at
13:50:43.483571Z and completed exit 0. Its completed journal SHA-256 is
3527134936cec5fbcee48d17db3fc8325bd8311a14167107920a37d0a3a64944.
Independent operation 8b7c59ff-d144-4a6d-9244-e2b73e7db6ee confirmed the loaded
canonical launcher, unchanged original files/profile and still-running old owner.

The normal-exit plan bound SHA-256
29e19826c9d94a0af20650234a81a8564489ad4f2d5548bc613de9c9f43c1a69.
Its one apply, operation 694e4c92-6948-4e02-b4fd-89c5a019111c, started
13:53:07.218679Z and completed exit 0: exact GUI/background pidfds observed exit,
KWin normal close and one SIGINT, profile preserved, not idle-only. Subsequent
readback confirmed both original PID directories absent, systemd inactive,
background endpoint absent and all stopped locks free. The conservative status
wrapper reports unconfirmed after its ELF verifier wraps an absent-PID exception;
that was not used as proof of absence or as a reason to repeat the quit.

The first real installation plan refused before transaction creation because the
installed unit is the exact package stock %h template, while fresh.payloads now
renders a canonical profile. Readback 9a3f3c94-cb0f-4ebc-9731-f19940a99135 proved
launcher/desktop/icon equality and stock-unit byte equality. Installed unit SHA-256
is 5658a9013f0bf01a6baede3c6581e087359f72a70f90e12dfc2cf5b1504c4601;
the rendered form is 22a48709eb12b56194a69e28b585a9e7d792dd2739a73284c1258cf83f0f8862.
The existing fixed public-profile wrapper makes retaining that exact stock unit
safe without editing it.

The adapter now recognizes only this precise retained-stock case: actual loaded
canonical launcher plus completed/no-pending authorization, and exact installed
unit digest equal to the already verified ZIP's stock template. Unknown custom
units, unverified/not-loaded wrappers and changes to the other three entries
still refuse. The plan records the four entry decisions; recovered records bind
the service exception to the exact fixed launcher arguments. Old unit bytes stay
pinned before/after exchange and rollback.

Four additional isolated cases plus the existing upgrade cases passed **45/45**
on HO5, operation 07632a76-7759-4955-9ef3-91b9539d8a83. Independent logic review
found no new blocker under these complete conditions; it did not independently
observe HO5 or claim installation. Only this necessary adapter increment was
edited through version-checked Octendra changes in HO5's existing primary main
while Studio's executor repeatedly disconnected. No new checkout/branch, product
rebuild or published-package mutation was introduced. Installation and actual
runtime/media results remain to be recorded separately.

## HO5 installation result and pending runtime acceptance

The strict stock-unit adapter was committed on HO5 main as
c8892dc2794932426dd9320dd4c79bae93aaa557. The prior push operation
9791918d-e6ae-4eb0-ac93-0f32350cbc44 timed out; a read-only remote ref
check c53ee903-f974-4d7e-9679-28df21bcb1ed showed main still at 643fd6b.
One retry d96b808e-5fb9-4286-9eeb-90cef50ff00e also timed out; read-only
07c88932-6080-4621-9461-200db54b6b7a again showed remote main at 643fd6b.
Do not infer that c889 is pushed or repeat a timed-out push without reading the
remote ref first.

The final real plan d92adf28-7a50-432c-9625-509fac9660ae was applicable
and bound SHA-256 776c6bba63dc2a0b45d99c87dae24862ad5eeccbca4a261616ba54d022905bf9.
Its one apply, operation b392d10d-0810-44ed-9188-f5868b827648, completed
exit 0 as transaction 22ae314a-2e76-4b1e-a5d7-b67b72e04937. The installed
ELF is source 81d772c7389d9e7f8deea03fb5e4f3dc55a9b774, SHA-256
5aef25ca02fc7664ef6412d7466f3e32395f391f017984854905d961f685285a.
It kept original profile/startup configuration and a rollback image.
Independent status 4eb292ad-6808-41e3-a300-f4d5e277d579 verified phase
completed, live_side=new, slot_state=verified, rollback_tuple_verified=true,
pending_metadata=false, no owners and inactive supervisor. Direct installed ELF
hash read 97dcc089-aa30-423a-b68f-7f85a9d5d2fe matched. Actual
--product-info-json read deaba61d-4c2d-465c-bbec-37d20e737207 confirms the
compiled original GPUI and native Linux video feature, not actual presentation.

The first two normal `systemctl --user start weavelo-desktop.service` requests
were rejected by automatic review without execution. The delegated thread then
received direct relevant authorization for ordinary HO5 Linux upgrade and service
restart. After independent state readback cfc827d9-dc0d-44ec-9f09-ac6b9e82c960,
one same-action retry 0cc1df44-54c3-48a4-aadb-353cb5c440f9 completed exit 0.
Systemd readback dfd9455e-b21f-43a4-acd5-e2a6a9136045 showed active/running,
MainPID 1772859 and the exact canonical profile launcher. /proc running-image
check 8f50a14a-4eba-476b-a6a0-a8cff79a5c67 matched the installed release
SHA-256. Product status 9e3cbe5c-d35f-45b5-9f10-a0ba8fa56111 confirmed
role background-service, the same PID, image hash and canonical profile.

The installed Desktop menu entry was invoked once by gtk-launch operation
b8bbbcbb-a0d0-472d-a81b-1e360b1c68b4 and returned 0, but immediate GUI
status/diagnostics were absent. With no GUI owner observed, one explicit normal
`--gui` launch d8635677-187e-4311-a0b0-8285de514d91 started PID 1779219.
Product status/diagnostics c3297de7-dc9d-4d8e-a000-343bc5fe0a12 then
confirmed the same PID/generation, new image hash, restored-original-gpui and
zero sessions. Targeted KWin metadata 7fcfe691-b356-429f-b541-e7e38d443835
confirmed one normal non-dialog Weavelo window for that PID. This is actual
window existence, not a visual screenshot or frame presentation proof. Background
and GUI diagnostics 4bbf3649-f6d2-45d6-acb5-22637ed825f6 reported relay
control and discovery both connected. Physical video, input, audio and long
session tests remain unknown. Formal computer-use inspection of the Studio remote
control app was denied, so no remote visual acceptance is claimed.

HO5 main additionally contains evidence commit 023d633d33d420a3133239fda606f78ede76fea3.
A dry-run push 6b26d1c2-6297-47b9-99b6-6b63be7a53c9 showed HO5 has no
usable GitHub write credential. Studio's existing checkout stayed clean at
643fd6b but its GitHub and gateway DNS failed. A verified 15,634-byte Git
bundle of the two commits was created with SHA-256
8497338addc40a4e6f4834054e5262a18a61f28c6f3f1149180f2f75320b66fd;
the supported file-download link could not be resolved from Studio. An attempted
terminal-output transfer was rejected by automatic review before execution and
must not be retried or routed around. Neither commit is claimed pushed or synced.

A secure invitation transport for the previously approved group move remains
unproven; no group credential, checkpoint or membership was changed.

## Menu-entry identity correction (read-only)

The earlier `gtk-launch fun.hackerlife.weavelo.desktop` used the **retained client**,
not the upgraded Desktop entry. The two installed application IDs coexist:

| Application ID passed to gtk-launch | Installed Exec | Binary SHA-256 |
| --- | --- | --- |
| `fun.hackerlife.weavelo.desktop` | `/var/home/liang/.local/opt/weavelo/weavelo` | `410fa94b28ceb134acdd3161e946d3dcc7240c995fba5df14455c14e198cb83f` |
| `fun.hackerlife.weavelo.desktop.desktop` | `/var/home/liang/.local/bin/weavelo-desktop` | `5aef25ca02fc7664ef6412d7466f3e32395f391f017984854905d961f685285a` (launcher target) |

GIO resolved both IDs to these distinct commands without launching either
(operation ec8cc2f2-9f53-428e-a09e-064187b94ded). The Desktop launcher
contains the canonical Desktop profile and executes the installed
`desktop/weavelo`; its read-only `--gui-status-json` returned PID 1779219,
original GPUI and the new ELF SHA (operation 045a96df-bf95-4afd-a881-4bcc3d2907fc).
The old top-level executable's same status command returned the old SHA. The
original menu result therefore did not test the new Desktop menu entry. Both
entries currently show the visible name `Weavelo`; identify them by application
ID/Exec for acceptance. No menu was clicked again, no new GUI was launched and
no installed entry was edited. A real click and rendered-frame acceptance remain
open.
