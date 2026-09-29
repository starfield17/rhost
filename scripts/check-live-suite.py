#!/usr/bin/env python3
"""Keep every frozen live scenario mapped to active Rust evidence."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    mapping = ROOT / "tests/live/legacy-map.tsv"
    rows = [line.split("\t") for line in mapping.read_text().splitlines()
            if line and not line.startswith("#")]
    if not rows:
        print("check-live-suite: tests/live/legacy-map.tsv has no frozen scenarios", file=sys.stderr)
        return 2
    evidence_paths = [ROOT / "tests/legacy_boundary.rs"]
    evidence_paths += sorted((ROOT / "tests/live").rglob("*.rs"))
    evidence_paths += sorted((ROOT / "tests/acceptance").rglob("*.rs"))
    evidence = "\n".join(path.read_text() for path in evidence_paths)
    for row in rows:
        legacy = row[0]
        rust = row[1].strip() if len(row) > 1 else ""
        if not rust:
            print(f"frozen scenario {legacy} has no Rust evidence symbol", file=sys.stderr)
            return 1
        if f"fn {rust}(" not in evidence:
            print(f"{legacy} maps to missing Rust test {rust}", file=sys.stderr)
            return 1
    ignored = []
    for path in [ROOT / "tests/live_exec.rs", *(ROOT / "tests/live").rglob("*")]:
        if path.is_file():
            for number, line in enumerate(path.read_text().splitlines(), 1):
                if re.search(r"#\[ignore", line):
                    ignored.append(f"{path.relative_to(ROOT)}:{number}:{line}")
    if ignored:
        print("\n".join(ignored))
        print("native live tests must be feature-gated as a suite, not individually ignored", file=sys.stderr)
        return 1
    print(f"check-live-suite: OK ({len(rows)} frozen scenarios mapped to Rust evidence)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
