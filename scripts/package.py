#!/usr/bin/env python3
"""Create a standalone source bundle with pinned local dependencies."""
import argparse
import io
import pathlib
import subprocess
import tarfile
import tempfile

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument("output",type=pathlib.Path)
args=parser.parse_args()
repo=pathlib.Path(__file__).resolve().parents[1]
client=repo.parent/"herdr-client-lib"
pin=(repo/"distribution/client-revision").read_text().strip()
def git(path,*args):return subprocess.check_output(["git","-C",str(path),*args])
def extract(path,revision,destination):
    destination.mkdir(parents=True,exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(git(path,"archive",revision)))as archive:
        archive.extractall(destination,filter="data")
with tempfile.TemporaryDirectory(prefix="herdr-source-")as scratch:
    root=pathlib.Path(scratch)/"herdr-tui"
    # The staged index permits checking packaging before the milestone commit.
    tree=git(repo,"write-tree").decode().strip()
    extract(repo,tree,root)
    ghostty=git(repo,"rev-parse",f"{tree}:vendor/ghostty").decode().strip()
    extract(repo/"vendor/ghostty",ghostty,root/"vendor/ghostty")
    extract(client,pin,root/"vendor/herdr-client-lib")
    manifest=root/"Cargo.toml"
    manifest.write_text(manifest.read_text().replace('path = "../herdr-client-lib"','path = "vendor/herdr-client-lib"'))
    # A vendored package must not accidentally join the TUI workspace.
    text=manifest.read_text().replace('[workspace]\n','[workspace]\nexclude = ["vendor/herdr-client-lib"]\n')
    manifest.write_text(text)
    (root/"SOURCE-REVISIONS").write_text(f"tui-tree {tree}\nclient {pin}\nghostty {ghostty}\n")
    args.output.parent.mkdir(parents=True,exist_ok=True)
    with tarfile.open(args.output,"w:gz")as archive:
        archive.add(root,arcname="herdr-tui")
print(args.output.resolve())
