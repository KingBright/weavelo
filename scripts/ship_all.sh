#!/usr/bin/env bash
# One-command build, package, and NAS publication entry for Weavelo.
# Supports selecting individual targets via flags, or defaults to all targets supported on this host.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
OS="$(uname -s)"

DO_MACOS=0
DO_LINUX=0
DO_WINDOWS=0
DO_ANDROID=0
DO_NAS_RELAY=0
UNSIGNED_ANDROID=0
BUILD_ONLY=0
DRY_RUN=0
EXPLICIT_TARGET=0

usage() {
  cat <<'EOF'
Usage: scripts/ship_all.sh [options]

Targets (if none specified, defaults to all targets supported on the current host):
  --macos              Build, sign, and package macOS arm64 desktop
  --linux              Build and package Linux x64 desktop (native Linux host)
  --windows            Build and package Windows x64 desktop (native Windows host)
  --android            Build, sign, and publish Android release APK (falls back to --unsigned-only if keystore absent)
  --unsigned-android   Build unsigned Android APK on build machine without signing key
  --nas-relay          Build and deploy isolated NAS relay binary

Modifiers:
  --build-only         Build and package locally without publishing to NAS
  --dry-run            Verify and dry-run publication without uploading
  -h, --help           Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --macos) DO_MACOS=1; EXPLICIT_TARGET=1 ;;
    --linux) DO_LINUX=1; EXPLICIT_TARGET=1 ;;
    --windows) DO_WINDOWS=1; EXPLICIT_TARGET=1 ;;
    --android) DO_ANDROID=1; EXPLICIT_TARGET=1 ;;
    --unsigned-android) DO_ANDROID=1; UNSIGNED_ANDROID=1; EXPLICIT_TARGET=1 ;;
    --nas-relay) DO_NAS_RELAY=1; EXPLICIT_TARGET=1 ;;
    --build-only) BUILD_ONLY=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage; exit 2 ;;
  esac
  shift
done

if [[ "$EXPLICIT_TARGET" == 0 ]]; then
  case "$OS" in
    Darwin)
      DO_MACOS=1
      DO_ANDROID=1
      ;;
    Linux)
      DO_LINUX=1
      ;;
    *)
      DO_WINDOWS=1
      ;;
  esac
fi

PUB_MODE=()
if [[ "$DRY_RUN" == 1 ]]; then
  PUB_MODE=(--dry-run)
fi

cd "$ROOT"

if [[ "$DO_MACOS" == 1 ]]; then
  echo "=== [1] Building macOS unified desktop package ==="
  bash "$ROOT/scripts/package_macos_unified.sh" --desktop
  if [[ "$BUILD_ONLY" == 0 ]]; then
    echo "=== [1] Publishing macOS package to NAS ==="
    if [[ -d "$ROOT/package/macos" ]]; then
      bash "$ROOT/scripts/publish_desktop_nas.sh" "$ROOT/package/macos" --macos-only "${PUB_MODE[@]}"
    else
      echo "Note: $ROOT/package/macos not found; verify packaging output directory." >&2
    fi
  fi
fi

if [[ "$DO_LINUX" == 1 ]]; then
  echo "=== [2] Building Linux x64 desktop package ==="
  bash "$ROOT/scripts/package_linux_desktop.sh"
  if [[ "$BUILD_ONLY" == 0 ]]; then
    echo "=== [2] Publishing Linux package to NAS ==="
    bash "$ROOT/scripts/publish_desktop_nas.sh" "$ROOT/package/linux" --linux-only "${PUB_MODE[@]}"
  fi
fi

if [[ "$DO_WINDOWS" == 1 ]]; then
  echo "=== [3] Building Windows x64 desktop package ==="
  python3 "$ROOT/scripts/build_windows_current.py"
fi

if [[ "$DO_ANDROID" == 1 ]]; then
  KEYSTORE="${WEAVELO_ANDROID_KEYSTORE:-$HOME/Library/Application Support/RemotePlay/Signing/remoteplay-release.p12}"
  if [[ "$UNSIGNED_ANDROID" == 1 ]] || [[ ! -f "$KEYSTORE" ]]; then
    echo "=== [4] Android signing keystore not present ($KEYSTORE); building --unsigned-only ==="
    bash "$ROOT/scripts/build_android_release.sh" --unsigned-only
    echo "Next: transfer unsigned APK to signing Mac and run scripts/build_android_release.sh --sign-prepared, or run scripts/sync_signing_bundle.sh to migrate keystore."
  else
    echo "=== [4] Building signed Android release APK ==="
    bash "$ROOT/scripts/build_android_release.sh"
    if [[ "$BUILD_ONLY" == 0 && "$DRY_RUN" == 0 ]]; then
      echo "=== [4] Publishing Android APK to NAS ==="
      bash "$ROOT/scripts/publish_android_nas.sh"
    fi
  fi
fi

if [[ "$DO_NAS_RELAY" == 1 ]]; then
  echo "=== [5] Building NAS relay ==="
  if [[ "$BUILD_ONLY" == 1 || "$DRY_RUN" == 1 ]]; then
    python3 "$ROOT/scripts/build_nas_relay.py" --build-only
  else
    python3 "$ROOT/scripts/build_nas_relay.py"
  fi
fi

echo "=== ship_all.sh finished ==="
