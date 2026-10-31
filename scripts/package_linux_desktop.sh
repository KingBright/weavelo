#!/usr/bin/env bash
# Build and package current main, or package an explicitly supplied native ELF.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ $# -eq 0 ]]; then
    exec python3 -B "$ROOT_DIR/scripts/release_artifact_lifecycle.py" run linux-desktop
fi
exec python3 -B "$ROOT_DIR/scripts/package_linux_desktop.py" "$@"
