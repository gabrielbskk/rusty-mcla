#!/usr/bin/env python3
"""Pack a converted Rusty MCLA install into independent ZIPs for phone download.

Each ZIP contains paths under RustyMCLA/. Extract every ZIP into the same
parent directory on Android, then select the resulting RustyMCLA directory.
No ZIP depends on another ZIP, so parts can be downloaded in any order.
"""

import argparse
import hashlib
import json
from pathlib import Path
from zipfile import ZIP_STORED, ZipFile

LIMIT = 90_000_000


def groups(root: Path):
    files = sorted(p for p in root.rglob("*") if p.is_file())
    if not (root / "manifest.local.json").is_file():
        raise SystemExit("Missing manifest.local.json: this is not a completed install")
    parts = []
    current = []
    size = 0
    for path in files:
        length = path.stat().st_size
        if current and size + length > LIMIT:
            parts.append(current)
            current = []
            size = 0
        current.append(path)
        size += length
    if current:
        parts.append(current)
    return parts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--part", type=int, help="1-based part number to create")
    args = parser.parse_args()
    root = args.root.resolve()
    parts = groups(root)
    summary = {
        "count": len(parts),
        "files": sum(len(g) for g in parts),
        "bytes": sum(p.stat().st_size for g in parts for p in g),
        "parts": [{"number": i + 1, "files": len(g), "bytes": sum(p.stat().st_size for p in g)} for i, g in enumerate(parts)],
    }
    if args.part is None:
        print(json.dumps(summary, indent=2))
        return
    if not 1 <= args.part <= len(parts):
        raise SystemExit(f"Part must be 1..{len(parts)}")
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"rusty-mcla-data-{args.part:02d}-of-{len(parts):02d}.zip"
    output = args.output / name
    with ZipFile(output, "w", compression=ZIP_STORED, allowZip64=True) as archive:
        for path in parts[args.part - 1]:
            archive.write(path, (Path("RustyMCLA") / path.relative_to(root)).as_posix())
    digest = hashlib.sha256()
    with output.open("rb") as stream:
        for block in iter(lambda: stream.read(4 * 1024 * 1024), b""):
            digest.update(block)
    print(json.dumps({"file": str(output), "bytes": output.stat().st_size, "sha256": digest.hexdigest()}))


if __name__ == "__main__":
    main()
