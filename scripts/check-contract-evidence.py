#!/usr/bin/env python3
"""Run the Rust contract evidence validator through the stable script entry."""
import os
from pathlib import Path
import subprocess
import sys


def main() -> int:
    args = sys.argv[1:]
    if args not in ([], ["-v"], ["--verbose"]):
        print(f"usage: {sys.argv[0]} [-v]", file=sys.stderr)
        return 2
    env = os.environ.copy()
    command = ["cargo", "test", "--locked", "--test", "contract_evidence"]
    if args:
        env["CONTRACT_EVIDENCE_VERBOSE"] = "1"
        command += ["--", "--nocapture"]
    else:
        command.append("--quiet")
    return subprocess.run(command, cwd=Path(__file__).resolve().parent.parent, env=env).returncode


if __name__ == "__main__":
    sys.exit(main())
