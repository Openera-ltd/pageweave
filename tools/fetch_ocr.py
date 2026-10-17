#!/usr/bin/env python3
"""Download the pinned OCR runtime (ONNX Runtime) and models, verify SHA-256, install atomically.

Usage: python3 -I tools/fetch_ocr.py [--platform linux-x64|win-x64] [--archive PATH]

Pins live in resources/ocr.lock.json (decision D-009). The runtime archive is verified before any
member is read, and only an allow-list of members is extracted (so archive links, debug symbols
and anything unexpected are never written). Each model file is verified after download. The
recogniser dictionary is a committed file whose hash is also checked. Installs into
third_party/ocr/<platform>/; an existing verified install is left untouched.
Exit codes: 0 ok, 2 bad arguments, 3 download failed, 4 checksum mismatch, 5 bad archive, 6 install failed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "resources" / "ocr.lock.json"
DEST_ROOT = ROOT / "third_party" / "ocr"
MAX_DOWNLOAD = 200 * 1024 * 1024
MAX_MEMBER = 100 * 1024 * 1024


def default_platform() -> str:
    machine = platform.machine().lower()
    if machine not in ("x86_64", "amd64"):
        raise SystemExit(f"unsupported architecture: {machine}")
    return {"Linux": "linux-x64", "Windows": "win-x64"}.get(platform.system()) or sys.exit(f"unsupported OS: {platform.system()}")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(url: str, target: Path) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": "pageweave-fetch-ocr"})
    total = 0
    with urllib.request.urlopen(req, timeout=120) as resp, target.open("wb") as out:
        while chunk := resp.read(1 << 20):
            total += len(chunk)
            if total > MAX_DOWNLOAD:
                raise ValueError("download exceeds size limit")
            out.write(chunk)


def extract_members(archive: Path, members: dict[str, str], staging: Path) -> None:
    """Copy only the listed regular-file members (archive name → install name)."""
    found: set[str] = set()
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as z:
            for info in z.infolist():
                if info.filename in members:
                    if info.is_dir() or info.file_size > MAX_MEMBER:
                        raise ValueError(f"unexpected member: {info.filename}")
                    with z.open(info) as src, (staging / members[info.filename]).open("wb") as dst:
                        shutil.copyfileobj(src, dst)
                    found.add(info.filename)
    else:
        with tarfile.open(archive, "r:gz") as tar:
            for m in tar.getmembers():
                if m.name in members:
                    if not m.isfile() or m.size > MAX_MEMBER:
                        raise ValueError(f"member is not a regular file: {m.name}")
                    src = tar.extractfile(m)
                    assert src is not None
                    with src, (staging / members[m.name]).open("wb") as dst:
                        shutil.copyfileobj(src, dst)
                    found.add(m.name)
    missing = set(members) - found
    if missing:
        raise ValueError(f"archive is missing {', '.join(sorted(missing))}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--platform")
    ap.add_argument("--archive", type=Path, help="use an already-downloaded runtime archive (still verified)")
    args = ap.parse_args()
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    plat = args.platform or default_platform()
    art = lock["onnxruntime"]["artifacts"].get(plat)
    if art is None:
        print(f"error: no pinned runtime for platform {plat!r}", file=sys.stderr)
        return 2
    models = lock["models"]
    dictionary = models["dictionary"]
    dict_src = ROOT / dictionary["file"]
    if sha256_file(dict_src) != dictionary["sha256"]:
        print(f"error: {dictionary['file']} does not match its pinned hash", file=sys.stderr)
        return 4
    fingerprint = hashlib.sha256(json.dumps([art["sha256"], {k: v["sha256"] for k, v in models["files"].items()}, dictionary["sha256"]]).encode()).hexdigest()

    dest = DEST_ROOT / plat
    marker = dest / ".verified"
    if marker.is_file() and marker.read_text().strip() == fingerprint:
        print(json.dumps({"status": "already_installed", "platform": plat, "path": str(dest)}))
        return 0

    DEST_ROOT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=DEST_ROOT, prefix=".fetch-") as tmp:
        tmpdir = Path(tmp)
        archive = tmpdir / Path(art["url"]).name
        try:
            if args.archive:
                shutil.copyfile(args.archive, archive)
            else:
                download(art["url"], archive)
        except Exception as e:  # network/HTTP/size errors
            print(f"error: runtime download failed: {e}", file=sys.stderr)
            return 3
        digest = sha256_file(archive)
        if digest != art["sha256"]:
            print(f"error: runtime checksum mismatch (expected {art['sha256']}, got {digest})", file=sys.stderr)
            return 4
        staging = tmpdir / "staging"
        staging.mkdir()
        try:
            extract_members(archive, art["members"], staging)
        except (ValueError, tarfile.TarError, zipfile.BadZipFile) as e:
            print(f"error: unsafe or corrupt runtime archive: {e}", file=sys.stderr)
            return 5
        archive.unlink()
        for name, f in models["files"].items():
            target = staging / name
            try:
                download(f["url"], target)
            except Exception as e:  # noqa: BLE001
                print(f"error: model download failed ({name}): {e}", file=sys.stderr)
                return 3
            got = sha256_file(target)
            if got != f["sha256"]:
                print(f"error: model checksum mismatch for {name} (expected {f['sha256']}, got {got})", file=sys.stderr)
                return 4
        shutil.copyfile(dict_src, staging / "rec_dict.txt")
        marker_tmp = staging / ".verified"
        marker_tmp.write_text(fingerprint)
        try:
            old = None
            if dest.exists():
                old = tmpdir / "previous"
                dest.rename(old)
            staging.rename(dest)
        except OSError as e:
            if old is not None and not dest.exists():
                old.rename(dest)
            print(f"error: install failed: {e}", file=sys.stderr)
            return 6
    print(json.dumps({"status": "installed", "platform": plat, "path": str(dest)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
