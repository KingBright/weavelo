#!/usr/bin/env python3
"""Cross-platform Weavelo desktop auto-updater.

Fetches the signed/hashed release manifest from NAS, compares against the
installed Weavelo binary's `--product-info-json` source commit, downloads and
verifies SHA-256, and delegates installation to the platform's existing
transactional updater (never bypassing signature or identity checks).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BASE_URL = "https://weavelo.hackerlife.fun:8443/download/"
MAX_MANIFEST_BYTES = 128 * 1024
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


def detect_platform_key() -> tuple[str, list[str]]:
    system = platform.system().lower()
    machine = platform.machine().lower()
    if system == "darwin" and machine in ("arm64", "aarch64"):
        return "macos", ["Weavelo-desktop-macos-arm64-"]
    if system == "linux" and machine in ("x86_64", "amd64"):
        return "linux", ["Weavelo-desktop-linux-x64-"]
    if system == "windows" and machine in ("amd64", "x86_64"):
        return "windows", ["Weavelo-desktop-windows-x64-", "Weavelo-Windows-x64-"]
    raise SystemExit(f"Unsupported platform for auto-update: {system}/{machine}")


def default_installed_binary(plat: str) -> Path | None:
    home = Path.home()
    candidates = {
        "macos": [
            home / "Applications/Weavelo/Weavelo.app/Contents/MacOS/weavelo",
            Path("/Applications/Weavelo.app/Contents/MacOS/weavelo"),
        ],
        "linux": [
            home / ".local/share/weavelo/desktop/current/weavelo",
            home / ".local/bin/weavelo",
        ],
        "windows": [
            Path(os.environ.get("LOCALAPPDATA", "")) / "Weavelo/Desktop/weavelo.exe",
        ],
    }.get(plat, [])
    for c in candidates:
        if c.is_file():
            return c
    return None


def installed_commit(binary: Path | None) -> str | None:
    if binary is None or not binary.is_file():
        return None
    proc = subprocess.run(
        [str(binary), "--product-info-json"],
        text=True,
        capture_output=True,
        timeout=15,
    )
    if proc.returncode != 0:
        return None
    info = json.loads(proc.stdout)
    src = (info.get("build_identity") or {}).get("source") or {}
    if src.get("state") == "clean" and isinstance(src.get("commit"), str):
        return src["commit"]
    return None


def fetch_bytes(url: str, max_bytes: int) -> bytes:
    req = urllib.request.Request(url, headers={"Cache-Control": "no-cache"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        data = resp.read(max_bytes + 1)
    if len(data) > max_bytes:
        raise ValueError(f"Response from {url} exceeds {max_bytes} bytes")
    return data


def select_entry(manifest: dict, prefixes: list[str]) -> dict:
    if manifest.get("schema") != 1 or not isinstance(manifest.get("releases"), list):
        raise ValueError("Invalid desktop release manifest schema")
    matches = [
        e for e in manifest["releases"]
        if isinstance(e, dict)
        and isinstance(e.get("file"), str)
        and any(e["file"].startswith(p) for p in prefixes)
        and e["file"].endswith(".zip")
        and "/" not in e["file"]
        and "\\" not in e["file"]
    ]
    if len(matches) != 1:
        raise ValueError(f"Expected 1 matching release for {prefixes}, found {len(matches)}")
    entry = matches[0]
    if not HEX40.match(str(entry.get("source_commit", ""))):
        raise ValueError("Invalid source_commit in manifest")
    if not HEX64.match(str(entry.get("sha256", ""))):
        raise ValueError("Invalid sha256 in manifest")
    return entry


def download_verified(url: str, dest: Path, expected_sha256: str, expected_size: int | None) -> None:
    req = urllib.request.Request(url, headers={"Cache-Control": "no-cache"})
    h = hashlib.sha256()
    total = 0
    with urllib.request.urlopen(req, timeout=120) as resp, dest.open("wb") as out:
        while True:
            chunk = resp.read(1024 * 1024)
            if not chunk:
                break
            out.write(chunk)
            h.update(chunk)
            total += len(chunk)
    if expected_size is not None and total != expected_size:
        raise ValueError(f"Downloaded size {total} != expected {expected_size}")
    actual = h.hexdigest()
    if actual != expected_sha256:
        raise ValueError(f"Downloaded SHA-256 {actual} != expected {expected_sha256}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default=os.environ.get("WEAVELO_DOWNLOAD_BASE_URL", DEFAULT_BASE_URL))
    parser.add_argument("--signer-sha1", help="macOS signer SHA-1 to query signer-pinned manifest")
    parser.add_argument("--binary", type=Path, help="Override path to installed weavelo binary")
    parser.add_argument("--apply", action="store_true", help="Download and apply update if newer commit is available")
    parser.add_argument("--force", action="store_true", help="Apply even if installed commit matches manifest")
    args = parser.parse_args()

    plat, prefixes = detect_platform_key()
    base = args.base_url.rstrip("/") + "/"
    manifest_name = "desktop-releases.json"
    if plat == "macos" and args.signer_sha1:
        signer = args.signer_sha1.lower()
        if not HEX40.match(signer):
            raise SystemExit("Invalid --signer-sha1")
        manifest_name = f"desktop-releases-macos-{signer}.json"

    manifest = json.loads(fetch_bytes(base + manifest_name, MAX_MANIFEST_BYTES).decode("utf-8"))
    entry = select_entry(manifest, prefixes)

    binary = args.binary or default_installed_binary(plat)
    current = installed_commit(binary)
    up_to_date = current is not None and current == entry["source_commit"]

    status = {
        "platform": plat,
        "manifest": manifest_name,
        "installed_binary": str(binary) if binary else None,
        "installed_commit": current,
        "latest_version": entry.get("version"),
        "latest_commit": entry["source_commit"],
        "file": entry["file"],
        "sha256": entry["sha256"],
        "up_to_date": up_to_date,
    }
    print(json.dumps(status, indent=2, ensure_ascii=False))

    if not args.apply or (up_to_date and not args.force):
        return 0

    if plat == "macos":
        cmd = [sys.executable, str(ROOT / "scripts/desktop_in_app_update.py")]
        print(f"Delegating macOS signed update to: {' '.join(cmd)}", flush=True)
        return subprocess.run(cmd, cwd=ROOT).returncode

    with tempfile.TemporaryDirectory(prefix="weavelo-auto-update-") as tmp:
        archive = Path(tmp) / entry["file"]
        print(f"Downloading {base + entry['file']}...", flush=True)
        download_verified(base + entry["file"], archive, entry["sha256"], entry.get("size_bytes"))
        if plat == "linux":
            cmd = [
                sys.executable,
                str(ROOT / "scripts/upgrade_linux_desktop.py"),
                str(archive),
                "--expected-archive-sha256", entry["sha256"],
                "--expected-commit", entry["source_commit"],
                "--apply",
            ]
        else:
            cmd = [
                sys.executable,
                str(ROOT / "scripts/upgrade_windows_desktop.py"),
                "--expected-archive-sha256", entry["sha256"],
                "--expected-commit", entry["source_commit"],
                "--apply",
            ]
        print(f"Running transactional updater: {' '.join(cmd)}", flush=True)
        return subprocess.run(cmd, cwd=ROOT).returncode


if __name__ == "__main__":
    raise SystemExit(main())
