#!/usr/bin/env python3
"""Run the unchanged-app checks against an isolated app and core.

The app draws on a private headless Weston when one is installed, or on the
Wayland socket given with --display. App, core, and TUI state stay in a scratch
directory either way.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument("--app", default=shutil.which("herdr-app"))
parser.add_argument("--weston", default=shutil.which("weston"))
parser.add_argument("--display", help="absolute path of an existing Wayland socket to draw on")
args = parser.parse_args()
if not args.app or not (args.weston or args.display):
    parser.error("herdr-app and either weston or --display are required")
repo = Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="tui-app-") as scratch:
    root = Path(scratch)
    runtime = root / "runtime"
    runtime.mkdir(mode=0o700)
    display = args.display or "tui-app-test"
    env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(root / "config"),
               XDG_STATE_HOME=str(root / "state"), XDG_CACHE_HOME=str(root / "cache"),
               HERDR_APP_HOME=str(root / "app"), HC_SESSION="tui-app-fixtures",
               WAYLAND_DISPLAY=display, DISPLAY="", HERDR_APP_NO_SOUND="1",
               HERDR_TUI_DISABLE_SOUND="1", TUI_APP_TEST_ROOT=scratch)
    children = []

    def run(argv):
        result = subprocess.run(argv, env=env, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(f"{argv}: {result.stdout} {result.stderr}")
        return result.stdout

    try:
        if not args.display:
            with (root / "weston.log").open("w") as log:
                children.append(subprocess.Popen([args.weston, "--backend=headless", "--fake-seat", "--shell=kiosk-shell.so",
                                                  "--no-config", f"--socket={display}"], env=env, stdout=log, stderr=log))
            for _ in range(100):
                if (runtime / display).exists():
                    break
                time.sleep(.1)
        with (root / "app.log").open("w") as log:
            children.append(subprocess.Popen([args.app], env=env, stdout=log, stderr=log))
        for _ in range(150):
            if subprocess.run([args.app, "status"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                break
            if children[-1].poll() is not None:
                raise RuntimeError((root / "app.log").read_text())
            time.sleep(.1)
        else:
            raise RuntimeError("app did not start: " + (root / "app.log").read_text())
        run([args.app, "do", "space.new", json.dumps(
            {"name": "fixture", "folder": scratch, "argv": ["sh", "-c", "printf APP_TUI_SHARED; cat"]})])
        address = None
        for _ in range(50):
            listed = json.loads(run(["hc", "ls", "--json"]))
            records = listed["terminals"] if isinstance(listed, dict) else listed
            address = next((t["address"] for t in records if t.get("where", "").startswith("fixture")), None)
            if address:
                break
            time.sleep(.1)
        if not address:
            raise RuntimeError(f"fixture terminal missing: {listed}")
        test_env = dict(env, TUI_APP_TEST_TERMINAL=address)
        for test in ("app_terminal.py", "workspace_app.py", "agents_app.py"):
            subprocess.run(["python3", str(repo / "tests" / test)], cwd=repo, env=test_env, check=True, timeout=300)
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        subprocess.run(["hc", "server", "stop"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
