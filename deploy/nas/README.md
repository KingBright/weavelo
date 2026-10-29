# Weavelo NAS delivery

The independent Weavelo relay is deployed on the NAS as
`weavelo-relay.service`, running `/opt/weavelo-relay/weavelo-relay` on
`127.0.0.1:39493`. Its public endpoint is
`wss://weavelo.hackerlife.fun:8443/v1/relay`. The older relay on port 39491
belongs to the retained deployment; it is not the Weavelo build or update target.

The relay routes authenticated device envelopes within the paired group.
The group secret remains on the endpoints. LAN and direct P2P remain preferred.

## Build and compatible update

Build from the primary main checkout with the existing producer:

```bash
python3 -B scripts/build_nas_relay.py
```

NAS receives verified runtime artifacts and metadata. Do not send source trees,
build caches or compilers there. Preserve existing service policy, credentials
and other applications. Validate the intended Caddy configuration before a reload,
then verify the running relay SHA, public TLS/WebSocket connection and device
traffic. Normal updates do not require new timestamped backups or manual patch
hash approval.

`Caddyfile.weavelo` provides the Weavelo hostname, relay upstream and a read-only
`/download/` route rooted at `/opt/weavelo/downloads`. Reconcile only that host
block with the active configuration; retain its existing authorized TLS setup.
Never copy DNS credentials into this repository or command output. Keep them in
root-readable service configuration; an exposed credential needs an authorized
rotation.

## Signed downloads

The public base is `https://weavelo.hackerlife.fun:8443/download/`.
Android metadata uses `release.json` and `versions.json`; desktop metadata uses
`desktop-releases.json`. Versioned artifacts are immutable. Latest metadata is
updated only after the corresponding package has passed its existing checks.

Publish the producer's current signed APK on its build machine:

```bash
./scripts/publish_android_nas.sh android/app/build/outputs/apk/release/app-release.apk
```

The Android filename and metadata take their source commit from the matching
compiler receipt, so a later publisher checkout cannot relabel an existing APK.
The publisher verifies the NAS copy and downloads the public APK to independently
check its SHA. Desktop publication uses `scripts/publish_desktop_nas.sh` and its
existing signature, identity and artifact checks.
