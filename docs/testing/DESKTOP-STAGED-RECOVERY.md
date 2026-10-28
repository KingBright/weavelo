# Retain an obsolete Desktop stage before a normal upgrade

`scripts/upgrade_macos_desktop.py --resume-staged` needs the exact original signed
archive. If that archive is unavailable, the original `staged` record previously
blocked every later upgrade even though no application had been stopped.

The project upgrader now supports `--retire-staged`. It accepts only the observed
upgrade-record SHA-256 and all three current installation pins. The record must
still be `staged`, without stop/runtime/rollback fields. Both the installed and
staged applications must pass the existing pinned signing and product-identity
checks. A stop or exchange phase cannot use this entry.

First produce a read-only plan:

```sh
python3 scripts/upgrade_macos_desktop.py --retire-staged \
  --expected-upgrade-sha256 OLD_JOURNAL_SHA256 \
  --expected-current-executable-sha256 CURRENT_EXECUTABLE_SHA256 \
  --expected-current-install-sha256 CURRENT_INSTALL_RECORD_SHA256 \
  --expected-current-plist-sha256 CURRENT_LAUNCH_PLIST_SHA256
```

Adding `--apply` performs only this recovery. Under the existing installer lock it
retains one fixed private `.desktop-upgrade.retired` slot beside the canonical App:

- The exact original journal bytes and a bound recovery receipt.
- The signed current App, original installation record and launch plist.
- The original staged directory, moved intact after the current backup is verified.

Only then does a compare-and-swap replace the active journal with the truthful
terminal phase `retired_before_stop`. The normal upgrader accepts that phase only
after checking the retained evidence. It carries the recovery reference into the
next upgrade and installation record. The prior rollback slot remains untouched;
this receipt does not claim that the staged version was ever installed.

Recovery never stops/starts an owner, changes a profile or permission, removes a
stage, recreates the missing archive or installs an App. Run the existing normal
signed upgrade separately, with its normal archive/source/current pins and the
previously authorized rollback choice. The recovery slot is never overwritten or
pruned. An exact interruption after the stage move can finish the journal commit;
an incomplete backup or unknown slot is retained and rejected for inspection.

Affected fixture regression:

```sh
python3 -B -m unittest discover -s scripts/tests -p test_upgrade_macos_desktop.py -v
```

These disposable tests cover evidence retention, exact interruption recovery,
phase/pin/signature/link/lock rejection, partial backup preservation and a normal
upgrade after recovery. They do not operate on a live service or system permissions.
