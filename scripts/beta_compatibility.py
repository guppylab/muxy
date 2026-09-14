#!/usr/bin/env python3
"""Check the beta compatibility declaration against the wire fixtures and a base commit."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
RECORD = "crates/muxy-protocol/beta-compatibility.json"


def identifier(root=ROOT):
    source = (root / "crates/muxy-protocol/src/build.rs").read_text()
    return int(re.search(r"pub const COMPATIBILITY: u64 = (\d+);", source)[1])


def current(root=ROOT):
    return {
        "identifier": identifier(root),
        "wire": {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                 for p in sorted((root / "crates/muxy-protocol/tests/fixtures").iterdir()) if p.is_file()},
    }


def validate(previous, actual):
    if actual["identifier"] < 1:
        raise ValueError("Compatibility identifiers must be positive")
    if previous and actual["identifier"] < previous["identifier"]:
        raise ValueError("Compatibility identifiers must not be reused")
    if previous and previous["identifier"] == actual["identifier"] and previous != actual:
        raise ValueError("Wire or behavior changed: bump COMPATIBILITY before updating the declaration")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--base")
    args = parser.parse_args()
    actual = current()
    record = ROOT / RECORD
    if args.write:
        record.write_text(json.dumps(actual, indent=2) + "\n")
        return
    if json.loads(record.read_text()) != actual:
        raise SystemExit("Beta compatibility declaration is stale. Review compatibility, then run scripts/beta_compatibility.py --write")
    if args.base and set(args.base) != {"0"}:
        previous = subprocess.run(["git", "show", f"{args.base}:{RECORD}"], cwd=ROOT, capture_output=True, text=True)
        if previous.returncode == 0:
            validate(json.loads(previous.stdout), actual)
        else:
            subprocess.run(["git", "cat-file", "-e", f"{args.base}^{{commit}}"], cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
