# Media lifecycle review, 2026-10-25

Reviewed the primary `main` checkout at `4d41b3c7890ec29ab0845bbc362f9c7a0777a852`.
The worktree was clean before this change, with no second source checkout or
Weavelo producer observed. A later Forge test held the shared Cargo build lock;
Weavelo validation waits for that lock and does not stop the other producer.

## Existing implementation and remaining scope

| Boundary | Current source evidence |
| --- | --- |
| Consumer demand | `remote_core/src/media_demand.rs` merges video/audio independently and fences replaced consumer callbacks by generation. The desktop model uses this registry. |
| Retained transport | Desktop demand changes use the existing `SetActivity` revision and acknowledgement path; they do not close the workspace connection. |
| Recovery | `video_recovery.rs`, workspace ingress and client payload-boundary tests already fence stale sources/decoder attempts and resumed packets. |
| macOS capture | Video and system-audio `pause` retain the SCStream owner and discard obsolete payload; actual stop remains separate. This is source evidence, not a new physical measurement. |
| Linux/Windows encoders | Existing pause branches still release the raw/native encoder or FFmpeg source and recreate it on resume. Cross-platform retained-encoder acceptance is incomplete. |
| Service ownership | `app/src/service_contract/session.rs` explicitly remains an executable adapter contract. The GUI session still owns receiver resources and unsubscribes on Drop; core-service migration is not complete. |

The 2026-10-14 plan's dated “not implemented” status must not be interpreted as
absence of the later partial implementation. This review does not authorize or
perform a new service architecture, native-resource transport, or deployment.

## Targeted correction

When a background consumer kept both media payloads active, deselecting a GUI
view left aggregate demand unchanged. The desktop model therefore skipped the
activity transition that otherwise released held input, and its input predicate
did not independently require selection.

The model now releases that view's held input on deselection and requires the
view to be selected before admitting input. It preserves the other consumer's
payload demand, activity revision and connection. The regression exercises the
actual desktop model and a loopback workspace actor using synthetic paint
eligibility, without a native window, screen capture or OS input injection.

## Validation

- `PYTHONDONTWRITEBYTECODE=1 ./scripts/test_release.sh`: 805 tests, successful,
  23 conditionally skipped. The final run uses the canonical existing temporary
  directory and ordinary process observation; signed integration is not enabled.
- `computer-use-input`: the guarded desktop-model build reached its existing
  1800-second deadline after shared-cache waiting and dependency compilation.
  Exit 124, zero tests executed, source hashes unchanged, post-observation exit
  zero and owned process group confirmed absent. This is not a passing Rust test.
  Receipt: `.rust-storage/validation-logs/computer-use-input-ui0ed0_a.jsonl`.
- `clock-workspace`: 25 passed, zero failed/ignored; source hashes unchanged,
  post-observation exit zero and owned process group confirmed absent.
  Receipt: `.rust-storage/validation-logs/clock-workspace-zj3qf9hq.jsonl`.
- `media-lifecycle-boundaries`: cancelled while Cargo was still waiting behind
  Forge Clippy (PID 27051) and another Forge test. Exit 130, zero tests executed,
  source hashes unchanged, post-observation exit zero and owned process group
  confirmed absent. No other producer was stopped or cache copied.
  Receipt: `.rust-storage/validation-logs/media-lifecycle-boundaries-undt2qxm.jsonl`.

### Follow-up after the shared target became idle

The two unfinished modes were resumed with their original 1800-second build and
120-second test deadlines, using the same cache after observing that Forge had
released it. The previous 805-test fixture suite and 25 core session tests were
not repeated; their scopes were unchanged.

The first desktop execution compiled successfully but reported two passes and
three fixture failures before the target assertions. The recorder advertised
`files=false` but used `WorkspaceConnection::connect`, which correctly rejected
an unpaired native file session. The test-only fixture now uses the existing
`connect_media` API and asserts that file capability is unavailable and its file
command channel is closed. No production authentication rule, device group,
permission or connection API was changed.

- Failed fixture receipt: `.rust-storage/validation-logs/computer-use-input-holpd1a0.jsonl`.
- Corrected `computer-use-input`: **5 passed, zero failed/ignored**, including
  `deselected_view_releases_input_while_another_consumer_keeps_payload_active`.
  Receipt: `.rust-storage/validation-logs/computer-use-input-lrspctfj.jsonl`.
- `media-lifecycle-boundaries`: **3 passed, zero failed/ignored**: retained resume
  IDR, watch/packet-ready ordering, and pause/resume through a full audio queue.
  Receipt: `.rust-storage/validation-logs/media-lifecycle-boundaries-59pp0_wf.jsonl`.

Both successful follow-up receipts confirm unchanged source hashes, successful
post-observation and absence of their owned process groups. They validate the
desktop model and synthetic payload queues, not cross-platform native resource
retention or physical presentation.

## Deployment assessment

The initial test-only stage did not create a release candidate. Its earlier
repository-relative ZIP check was insufficient: Cargo resolves production macOS
output to `~/rust-target/package/macos`, and the independent Desktop namespace
uses `Weavelo-desktop-macos-arm64.zip`.

### Formal candidate follow-up

The signing-machine handoff described in this historical stage is superseded by
the user's subsequent explicit Studio-new-identity approval. Current signing
status and remaining native authorization are recorded in
[STUDIO-DESKTOP-LOCAL-SIGNING.md](STUDIO-DESKTOP-LOCAL-SIGNING.md); no Book
continuation is required by that route.

Both native builds below consumed clean primary `main` source
`d722c5a9a2a974e1ceb0ab7e0712f699fe11e021`, snapshot
`3f52d939469af087f0885b9490c8fd2125e998e5f2983a4fd30c9cc29ae4a409`.
The fixed `build_identity_gate.py build` entry reused each platform's existing
cache. Both returned `observed-build` receipts, and separate queries executed
the exact compiler outputs with `--product-info-json`, matching their receipts.
They confirm original GPUI and native video compilation, not physical acceptance.

| Evidence | Mac Studio | Linux HO5 |
| --- | --- | --- |
| Binary SHA-256 | `87769a12b4bbaa1e6487b0044aef5fbc51ba202ce738bc8e468f5ea0eb310b22` | `4235d63da42d73401de7cb1112885fadbfbe5364ac91ae8b45a84fdd6cf839ca` |
| Build receipt SHA-256 | `430db7f1839ca5e66b24e3a6737a532a3ce5cd549f7448526ea28254be98a6b7` | `15ff7f50e43519597e36469ccd743862e3ab8fab572653bfc3a880e6152b190f` |
| Compiled identity SHA-256 | `5e415e9e1cdfc89b4eec10085e73c140d7e5fa1dfdb1732327f652fa2fd39908` | `7ec65be683eb183487502c2a86a151bc11b84163abd5088c9b65f84c0f9aef8d` |

On Studio, the release signing preflight **rejected** signing because the pinned
identity `8eaa97365d97b8a987cdf1232430eddf9aba8ecf` is unavailable in the existing
keychain search. The unrelated valid Remote Hosts certificate was not used.
No certificate was created/imported, no key copied, and no security setting
changed. The existing installed Desktop app independently passes its exact
certificate/designated-requirement check at version `2.0.0-alpha.9`, build
`20261023.16`, binary SHA-256
`c50d0cafc0cfa2055210fbe7565105c5580f15a95e1dc9f22256e2576ea76d9c`.

The existing explicit `package_macos_unified.sh --prepare-desktop` mode then
completed through the original public transaction. Its current handoff archive
is `~/rust-target/package/macos/Weavelo-desktop-macos-arm64.prepared.zip`,
10,630,622 bytes, SHA-256
`4911570d41ee66e922bd7dec90defff50349c21c247f8830a955027e8329fc55`.
Its verification receipt SHA-256 is
`8ae722b74d82ce93bc4814cf19b537f23c96e8f31652b1acb6443b06fb0083a7`.
Independent preparation verification passes; the marker explicitly states
`installable=false` and `signature_verified=false`. This is a handoff to the
existing authorized signing machine, not a signed-release substitute. The signed
Desktop ZIP remains absent. Completing signing requires that existing authorized
signing environment; changing the pinned identity is outside this work.

On HO5, the verified existing FFmpeg/Opus SDK bindings were included before
configuration preparation. Both its installed trusted Linux activity companion
and the current source hash to
`5f6d67612014d2bdfc945cca8aa9d241d9c4c73b98d207ae578cea2d44e057c6`.
The 2026-10-23 missing/metadata-only adapter report is historical: the later
scoped process, open-file and launch-reference implementation is present.
`bash scripts/package_linux_desktop.sh` completed normally through the sole
public release authority; `current_committed=true`, cleanup completed. The
initial direct wrapper invocation exited 126 before packaging: Git records the
wrapper as mode 100644 and HO5 stores it as 0600. Calling its declared Bash
interpreter changed no file permissions, mount options or validation rules.

The current Linux archive is
`/var/home/liang/workspace/remote_play/target/package/linux/Weavelo-desktop-linux-x64.zip`,
18,245,308 bytes, SHA-256
`f73f7ac56331877c2946e4dc76e11b6f8e03c062c333f6de15606a8c53c5a907`.
Its manifest SHA-256 is
`be35b5c5fdceea31a0ee3025b19b7ea8b26e441ca87d09c7d5ac996a638fd5b3`,
verification receipt SHA-256
`48a0135b78190e055975d5fe36877a4367c53b33a32fe73a3baaf57f9a437f03`.
An independent archive/manifest verification passes, with no pending output
paths. It is the existing unsigned Linux package type; no service was started.

HO5 and cube primary workspaces were clean and fast-forwarded from `5d3a16b`
to `d722c5a9`, with no new checkout or protected WIP overwritten. The shared
identity entry explicitly supports native macOS/Linux only, but the earlier
conclusion that Windows has no current native producer was too broad:
`scripts/build_windows_current.py` already supplies a primary-main Windows
producer with the existing project-local cache, owned fixed identity inputs,
compiler environment binding and actual ProductInfo verification. Its
`--build-only` mode does not invoke PowerShell packaging or change policy.

The follow-up corrects a concrete producer/packager mismatch: the producer now
passes `-Desktop` to the existing packager and reports its actual independent
`target/package/windows/desktop/Weavelo-Windows-x64.zip` output. It rejects an
unknown or retained compiled namespace before minting the current receipt.
Previously it omitted the flag, causing the packager to reject the independent
binary, and reported the retained package path. The retained output is not
selected by the corrected invocation.

Three fixed synthetic-compiler fixtures drive the actual Python entry to check
Desktop routing/receipt binding, namespace rejection and build-only separation.
The focused Windows suite runs 14 tests with one native-Windows conditional
skip. The required `./scripts/test_release.sh` suite then runs 808 tests with 23
conditional skips, zero failures. Neither suite compiles or packages the real
Windows product; cube is offline and no native acceptance is claimed. No
execution-policy setting or installed application was changed.

The Windows PowerShell packager still lacks the original public current-output
transaction: its existing implementation removes old output before assembling
the replacement. This separate delivery gate is not repaired by the namespace
fix. A complete integration needs Windows implementations of the original
helper's producer locks and scoped activity/reference checks, then the existing
packager must consume `release-admit` staging and publish only after its original
verification, preserving old current bytes on failure. This requires a reviewed
native helper adaptation, not a second ledger, raw ZIP writer or alternate cache.
No new Windows package was attempted during this follow-up.

Small independent evidence is retained under `.rust-storage/validation-logs/`:
`d722c5a9-macos-release-20261025.observed-build.json`,
`d722c5a9-macos-preparation-independent-evidence-20261025.json`, and
`d722c5a9-linux-package-independent-evidence-20261025.json`.
The Linux native build receipt remains in the same evidence directory on HO5.
Both producer transactions ended; Mac fixed identity inputs were removed by
their owner and the identity state is empty. No cache cleanup was performed.

Not eligible for deployment from this run. No NAS publication or application
installation was performed. The Studio-new-identity follow-up has now completed
the signed Mac candidate (build `20261025.9`), recorded in
[STUDIO-DESKTOP-LOCAL-SIGNING.md](STUDIO-DESKTOP-LOCAL-SIGNING.md). Windows package
transaction/native acceptance and physical media acceptance remain separate from
the successful Mac/Linux candidates and the existing Windows build-only entry.

Physical acceptance remains separate: two-Mac pause/resume resource identities,
fresh actual presentation and reliable input, controlled audio without replay,
long-duration behavior, and native HO5/cube behavior have not been measured in
this run. No installed application or published package is changed.
