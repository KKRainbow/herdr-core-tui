#!/usr/bin/env python3
"""Collect dependency license notices from the locked Cargo graph."""
import json,pathlib,subprocess
root=pathlib.Path(__file__).resolve().parents[1]
metadata=json.loads(subprocess.check_output(["cargo","metadata","--locked","--offline","--filter-platform","x86_64-unknown-linux-gnu","--format-version","1"],cwd=root))
lines=["# Third-party notices","","Ghostty retains its license and notices in `vendor/ghostty`.","Adapted Herdr sources retain Apache-2.0 licensing; see `docs/provenance.md`.","The release source bundle includes the pinned library license.","","| Package | Version | License |","|---|---|---|"]
licenses=root/"distribution/licenses"
licenses.mkdir(parents=True,exist_ok=True)
for package in sorted(metadata["packages"],key=lambda p:(p["name"],p["version"])):
 if not package.get("source"):continue
 lines.append(f'| {package["name"]} | {package["version"]} | {package.get("license") or "See source"} |')
 directory=pathlib.Path(package["manifest_path"]).parent
 for file in directory.iterdir():
  if file.is_file() and file.name.lower().startswith(("license","licence","copying","notice")):
   (licenses/(package["name"]+"-"+package["version"]+"-"+file.name)).write_bytes(b"\n".join(line.rstrip() for line in file.read_bytes().splitlines()).rstrip()+b"\n")
(root/"THIRD_PARTY.md").write_text("\n".join(lines)+"\n")
