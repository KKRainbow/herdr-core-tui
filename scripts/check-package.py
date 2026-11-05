#!/usr/bin/env python3
"""Build and test the exported source without sibling checkout dependencies."""
import json
import pathlib
import subprocess
import tempfile

repo = pathlib.Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="herdr-package-check-") as scratch:
    scratch = pathlib.Path(scratch)
    bundle = scratch / "source.tar.gz"
    subprocess.run(["python3", str(repo / "scripts/package.py"), str(bundle)], check=True)
    subprocess.run(["tar", "-xzf", str(bundle), "-C", str(scratch)], check=True)
    source = scratch / "herdr-tui"
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--no-deps"], cwd=source
    ))
    client = next(d for p in metadata["packages"] for d in p["dependencies"] if d["name"] == "herdr-client-lib")
    assert pathlib.Path(client["path"]).resolve() == source / "vendor/herdr-client-lib"
    subprocess.run(["cargo", "build", "--locked"], cwd=source, check=True)
    subprocess.run(["cargo", "test", "--all-targets", "--locked"], cwd=source, check=True)
