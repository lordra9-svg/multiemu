#!/usr/bin/env python3
"""Builds deterministic release archives from an existing release build.

Usage:
    scripts/package.py --platform linux --arch x86_64
    scripts/package.py --platform windows --arch x86_64
    scripts/package.py --platform macos --arch arm64

The target binaries must already be built under `target/release` (or the
directory passed with --target-dir). Archives contain fixed timestamps, sorted
entries and zeroed ownership, so two builds of the same commit produce
byte-identical output. `SOURCE_DATE_EPOCH` selects the timestamp; it defaults
to the current git commit time.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import os
import platform
import shutil
import stat
import subprocess
import tarfile
import tempfile
import time
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"


def workspace_version() -> str:
    try:
        with open(ROOT / "Cargo.toml", "rb") as handle:
            return tomllib.load(handle)["workspace"]["package"]["version"]
    except (OSError, KeyError, tomllib.TOMLDecodeError):
        return "0.0.0"


def source_date_epoch() -> int:
    value = os.environ.get("SOURCE_DATE_EPOCH")
    if value:
        return int(value)
    try:
        return int(
            subprocess.run(
                ["git", "-C", str(ROOT), "log", "-1", "--format=%ct"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
        )
    except (OSError, subprocess.CalledProcessError, ValueError):
        return 0


def copy_file(source: Path, destination: Path, mode: int = 0o644) -> None:
    if not source.is_file():
        raise SystemExit(f"missing file: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(mode)


def copy_tree(source: Path, destination: Path) -> None:
    for path in sorted(source.rglob("*")):
        if path.is_file():
            target = destination / path.relative_to(source)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            target.chmod(0o644)


def info_plist(version: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleDisplayName</key><string>Unko</string>
    <key>CFBundleExecutable</key><string>unko-app</string>
    <key>CFBundleIconFile</key><string>unko.icns</string>
    <key>CFBundleIdentifier</key><string>dev.unko.app</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>Unko</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"""


def stage(platform_name: str, arch: str, target_dir: Path, staging: Path, version: str) -> str:
    stem = f"unko-{platform_name}-{arch}"
    root = staging / stem
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True)
    exe = ".exe" if platform_name == "windows" else ""

    if platform_name == "macos":
        app = root / "Unko.app" / "Contents"
        copy_file(
            target_dir / "release" / "unko-app",
            app / "MacOS" / "unko-app",
            0o755,
        )
        (app / "Info.plist").write_text(info_plist(version))
        icon = ASSETS / "unko.icns"
        if icon.is_file():
            copy_file(icon, app / "Resources" / "unko.icns")
        copy_file(target_dir / "release" / "unko", root / "unko", 0o755)
    else:
        copy_file(target_dir / "release" / f"unko{exe}", root / f"unko{exe}", 0o755)
        copy_file(
            target_dir / "release" / f"unko-app{exe}", root / f"unko-app{exe}", 0o755
        )

    if platform_name == "linux":
        copy_file(
            ASSETS / "unko.desktop",
            root / "share" / "applications" / "unko.desktop",
        )
        copy_file(
            ASSETS / "unko-256.png",
            root / "share" / "icons" / "hicolor" / "256x256" / "apps" / "unko.png",
        )

    copy_file(ROOT / "LICENSE", root / "LICENSE")
    copy_file(ROOT / "NOTICE", root / "NOTICE")
    copy_file(ROOT / "README.md", root / "README.md")
    copy_tree(ROOT / "licenses", root / "licenses")
    return stem


def tar_bytes(files: list[tuple[Path, str]], epoch: int) -> bytes:
    buffer = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=buffer, mtime=0) as gzip_file:
        with tarfile.open(
            fileobj=gzip_file, mode="w", format=tarfile.USTAR_FORMAT
        ) as archive:
            for path, name in sorted(files, key=lambda item: item[1]):
                data = path.read_bytes()
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mtime = epoch
                info.uid = 0
                info.gid = 0
                info.uname = ""
                info.gname = ""
                info.mode = 0o755 if os.access(path, os.X_OK) else 0o644
                archive.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def zip_bytes(files: list[tuple[Path, str]], epoch: int) -> bytes:
    buffer = io.BytesIO()
    stamp = time.gmtime(max(epoch, 315532800))[:6]  # zip needs >= 1980
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path, name in sorted(files, key=lambda item: item[1]):
            data = path.read_bytes()
            info = zipfile.ZipInfo(name, date_time=stamp)
            info.external_attr = (
                (0o755 if os.access(path, os.X_OK) else 0o644) | stat.S_IFREG
            ) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    return buffer.getvalue()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", required=True, choices=["linux", "windows", "macos"])
    parser.add_argument("--arch", default=platform.machine())
    parser.add_argument("--target-dir", default=str(ROOT / "target"))
    parser.add_argument("--out", default=str(ROOT / "dist"))
    parser.add_argument("--version", default=workspace_version())
    args = parser.parse_args()

    epoch = source_date_epoch()
    target_dir = Path(args.target_dir)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="unko-package-") as staging:
        stem = stage(args.platform, args.arch, target_dir, Path(staging), args.version)
        root = Path(staging) / stem
        files = [
            (path, path.relative_to(root).as_posix())
            for path in root.rglob("*")
            if path.is_file()
        ]
        prefix = f"{stem}/"
        files_with_prefix = [(path, prefix + name) for path, name in files]
        if args.platform == "windows":
            archive = zip_bytes(files_with_prefix, epoch)
            output = out / f"{stem}.zip"
        else:
            archive = tar_bytes(files_with_prefix, epoch)
            output = out / f"{stem}.tar.gz"
        output.write_bytes(archive)

    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    checksums = out / "SHA256SUMS"
    entries: list[str] = []
    for path in sorted(out.iterdir()):
        if path.name == "SHA256SUMS" or not path.is_file():
            continue
        entries.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}")
    checksums.write_text("\n".join(entries) + "\n")
    print(f"{output.name}: {len(archive)} bytes, sha256 {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
