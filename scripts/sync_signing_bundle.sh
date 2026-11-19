#!/usr/bin/env bash
# Encrypted one-command migration of existing Weavelo signing identities between owner Macs.
# Private keys and Keychain passphrases stay encrypted on disk and are prompted directly from /dev/tty.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
POLICY_JSON="$ROOT/deploy/macos/signing-policy.json"
ANDROID_SIGN_DIR="${WEAVELO_ANDROID_SIGNING_DIR:-$HOME/Library/Application Support/RemotePlay/Signing}"
ANDROID_KEYSTORE="${WEAVELO_ANDROID_KEYSTORE:-$ANDROID_SIGN_DIR/remoteplay-release.p12}"
ANDROID_KEYCHAIN_SERVICE="com.remoteplay.android.release.signing"
ANDROID_KEYCHAIN_ACCOUNT="${USER:-remoteplay}"
LOGIN_KEYCHAIN="${WEAVELO_LOGIN_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"

usage() {
  cat <<'EOF'
Usage:
  scripts/sync_signing_bundle.sh status
      Inspect which macOS / Android signing identities are present on this Mac (read-only, no secrets).

  scripts/sync_signing_bundle.sh export <bundle.enc>
      Run on the source Mac (e.g. MacBook) to pack and AES-256-CBC (PBKDF2) encrypt:
        - Android release keystore (remoteplay-release.p12) + Keychain password
        - macOS release code-signing identity pinned in deploy/macos/signing-policy.json
      Transfer <bundle.enc> to Mac Studio via AirDrop / local SCP, then run `import`.

  scripts/sync_signing_bundle.sh import <bundle.enc>
      Run on the target Mac (e.g. Mac Studio) to decrypt, install the Android keystore & Keychain
      password, import the macOS signing identity into login Keychain, and verify both identities.
EOF
}

policy_sha1() {
  python3 - "$POLICY_JSON" <<'PY'
import json, sys
print(json.load(open(sys.argv[1]))["certificate_sha1"].upper())
PY
}

cmd_status() {
  local want_sha1
  want_sha1="$(policy_sha1)"
  echo "=== Weavelo Signing Status ($(hostname)) ==="
  echo "Policy macOS certificate SHA-1: $want_sha1"
  if security find-identity -v -p codesigning 2>/dev/null | grep -qi "$want_sha1"; then
    echo "  [OK] macOS policy identity present in Keychain"
  else
    echo "  [MISSING] macOS policy identity ($want_sha1) not found in default Keychain"
  fi
  if [[ -f "$ANDROID_KEYSTORE" ]]; then
    echo "  [OK] Android keystore present: $ANDROID_KEYSTORE"
  else
    echo "  [MISSING] Android keystore not found: $ANDROID_KEYSTORE"
  fi
  if security find-generic-password -a "$ANDROID_KEYCHAIN_ACCOUNT" -s "$ANDROID_KEYCHAIN_SERVICE" >/dev/null 2>&1; then
    echo "  [OK] Android Keychain password entry present ($ANDROID_KEYCHAIN_SERVICE / $ANDROID_KEYCHAIN_ACCOUNT)"
  else
    echo "  [MISSING] Android Keychain password entry ($ANDROID_KEYCHAIN_SERVICE / $ANDROID_KEYCHAIN_ACCOUNT)"
  fi
}

cmd_export() {
  local out="${1:?Usage: sync_signing_bundle.sh export <bundle.enc>}"
  local want_sha1 stage
  want_sha1="$(policy_sha1)"
  stage="$(mktemp -d "${TMPDIR:-/tmp}/weavelo-sign-export.XXXXXX")"
  chmod 700 "$stage"
  trap 'rm -rf -- "$stage"' EXIT

  echo "1/3 Exporting Android release keystore..."
  [[ -f "$ANDROID_KEYSTORE" ]] || { echo "Error: Android keystore not found at $ANDROID_KEYSTORE" >&2; exit 1; }
  cp "$ANDROID_KEYSTORE" "$stage/remoteplay-release.p12"
  chmod 600 "$stage/remoteplay-release.p12"
  umask 077
  security find-generic-password -a "$ANDROID_KEYCHAIN_ACCOUNT" -s "$ANDROID_KEYCHAIN_SERVICE" -w > "$stage/android-keychain-pass.txt"
  chmod 600 "$stage/android-keychain-pass.txt"

  echo "2/3 Exporting macOS signing identity ($want_sha1)..."
  echo "    macOS Keychain may prompt for a temporary .p12 export passphrase and Keychain confirmation."
  if security find-identity -v -p codesigning 2>/dev/null | grep -qi "$want_sha1"; then
    # Export identities from login keychain in PKCS#12 format (interactive passphrase via security/TTY)
    security export -k "$LOGIN_KEYCHAIN" -t identities -f pkcs12 -o "$stage/macos-identities.p12"
    chmod 600 "$stage/macos-identities.p12"
  else
    echo "Error: macOS policy identity $want_sha1 not found in $LOGIN_KEYCHAIN" >&2
    exit 1
  fi

  echo "3/3 Encrypting bundle to $out (enter a transfer passphrase)..."
  tar -C "$stage" -cf - remoteplay-release.p12 android-keychain-pass.txt macos-identities.p12 \
    | openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt -out "$out"
  chmod 600 "$out"
  echo "Done: $out"
  echo "SHA-256: $(shasum -a 256 "$out" | awk '{print $1}')"
  echo "Next: transfer $out to Mac Studio (AirDrop/SCP) and run:"
  echo "  bash scripts/sync_signing_bundle.sh import $out"
}

cmd_import() {
  local src="${1:?Usage: sync_signing_bundle.sh import <bundle.enc>}"
  [[ -f "$src" ]] || { echo "Error: encrypted bundle not found: $src" >&2; exit 1; }
  local stage android_pass
  stage="$(mktemp -d "${TMPDIR:-/tmp}/weavelo-sign-import.XXXXXX")"
  chmod 700 "$stage"
  trap 'rm -rf -- "$stage"; unset android_pass' EXIT

  echo "1/3 Decrypting $src (enter transfer passphrase)..."
  umask 077
  openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 -in "$src" | tar -C "$stage" -xf -

  echo "2/3 Installing Android keystore and Keychain entry..."
  mkdir -p "$ANDROID_SIGN_DIR"
  chmod 700 "$ANDROID_SIGN_DIR"
  cp "$stage/remoteplay-release.p12" "$ANDROID_KEYSTORE"
  chmod 600 "$ANDROID_KEYSTORE"
  android_pass="$(cat "$stage/android-keychain-pass.txt")"
  security add-generic-password -U \
    -a "$ANDROID_KEYCHAIN_ACCOUNT" \
    -s "$ANDROID_KEYCHAIN_SERVICE" \
    -w "$android_pass" >/dev/null
  unset android_pass

  echo "3/3 Importing macOS signing identity into $LOGIN_KEYCHAIN..."
  echo "    Enter the .p12 export passphrase you chose on the source Mac:"
  security import "$stage/macos-identities.p12" \
    -k "$LOGIN_KEYCHAIN" \
    -f pkcs12 \
    -T /usr/bin/codesign \
    -T /usr/bin/security

  echo ""
  cmd_status
  echo ""
  echo "Tip: after confirming both [OK] above, delete the temporary encrypted file: rm -f $src"
}

case "${1:-}" in
  status) cmd_status ;;
  export) shift; cmd_export "$@" ;;
  import) shift; cmd_import "$@" ;;
  -h|--help|"") usage ;;
  *) echo "Unknown command: $1" >&2; usage; exit 2 ;;
esac
