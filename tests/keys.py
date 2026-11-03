"""Herdr's remaining keybinds: indexed workspaces and agents, notification target, direct resize, scrollback editor, clear pane."""
import fcntl
import json
import os
import pathlib
import pty
import select
import struct
import subprocess
import termios
import time

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herder-tui"
core = Core("keys")
result = core.root / "scrollback.txt"
editor = core.root / "editor.sh"
editor.write_text(f"#!/bin/sh\ncp \"$1\" {result}\n")
editor.chmod(0o700)
core.env["EDITOR"] = str(editor)
config = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herder-tui"
config.mkdir(parents=True)
shell_result = core.root / "shell-command.txt"
(config / "config.toml").write_text(f"""
[keys]
resize_pane_right = "prefix+ctrl+l"
clear_pane = "prefix+y"
focus_agent = "prefix+alt+1..9"

[keys.indexed]
workspaces = "alt"

[[keys.command]]
key = "prefix+u"
command = {json.dumps(f"printf '%s %s' \"$HERDR_ACTIVE_PANE_ID\" \"$HC_SESSION\" > {shell_result}")}

[[keys.command]]
key = "prefix+i"
command = "echo pane-command; sleep 60"
type = "pane"
description = "pane command"
""")
block = "printf '\\033]7501;state=working:app=codex\\033\\\\'; sleep 3; printf '\\033]7501;state=blocked:app=codex\\033\\\\'; sleep 180"
process = master = slave = None
try:
    def do(command, args):
        return json.loads(subprocess.check_output([str(binary), "do", command, json.dumps(args)], env=core.env, timeout=20))

    def drain(seconds=.2):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .05)[0]:
                os.read(master, 65536)

    def state():
        out = subprocess.run([str(binary), "--instance", "keys", "view", "state", "{}"], env=core.env, capture_output=True, timeout=5)
        return json.loads(out.stdout)["view"]

    def until(predicate, what):
        end = time.monotonic() + 15
        while time.monotonic() < end:
            drain(.1)
            if predicate():
                return
        raise AssertionError(what)

    first = do("space.new", {"name": "First", "argv": ["sh"]})
    ordinary = "printf '\\033]7501;state=working:app=make\\033\\\\'; sleep 180"
    second = do("space.new", {"name": "Second", "argv": ["sh", "-c", ordinary]})
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "keys"], env=core.env, stdin=slave, stdout=slave, stderr=slave)
    drain(1.5)

    # alt+2 and alt+1 switch workspaces through [keys.indexed].
    os.write(master, b"\x1b2")
    until(lambda: state()["active_space"] == second["space"], "alt+2 did not switch workspace")
    os.write(master, b"\x1b1")
    until(lambda: state()["active_space"] == first["space"], "alt+1 did not switch workspace")

    # Agent focus skips the ordinary status reporter in the same workspace.
    agent = do("tab.new", {"space": second["space"], "argv": ["sh", "-c", block]})
    drain(5)
    os.write(master, b"\x1b1")
    until(lambda: state()["active_space"] == first["space"], "focus did not return")
    os.write(master, b"\x02o")
    until(lambda: state()["focused"] == agent["pane"], "prefix+o did not open the notification target")
    os.write(master, b"\x1b1")
    until(lambda: state()["active_space"] == first["space"], "focus did not return")
    os.write(master, b"\x02\x1b1")
    until(lambda: state()["focused"] == agent["pane"], "focus_agent:1 did not focus the agent")

    # Direct resize moves the divider without resize mode.
    os.write(master, b"\x1b1")
    until(lambda: state()["active_space"] == first["space"], "focus did not return")
    split = do("pane.split", {"id": first["pane"], "axis": "row", "argv": ["sh", "-c", "sleep 180"]})
    subprocess.run([str(binary), "--instance", "keys", "view", "pane.focus", json.dumps({"id": first["pane"]})], env=core.env, capture_output=True, timeout=5)
    drain(.5)
    def weight():
        tab = next(t for s in core.rpc("doc.get", {"name": "herdr-app"})["value"]["spaces"] for t in s["tabs"] if t["id"] == first["tab"])
        return next((c.get("size") or {}).get("weight", 1) for c in tab["root"]["children"] if c["id"] == first["pane"])
    before = weight()
    os.write(master, b"\x02\x0c")
    until(lambda: weight() > before, "prefix+ctrl+l did not grow the pane")

    # Scrollback opens in $EDITOR; clear pane drops it for this viewer.
    os.write(master, b"for i in 1 2 3; do echo marker-$i; done\r")
    drain(1)
    os.write(master, b"\x02e")
    until(lambda: result.exists() and "marker-3" in result.read_text(), "prefix+e did not open scrollback")
    result.unlink()
    os.write(master, b"\x02y")
    drain(.5)
    os.write(master, b"\x02e")
    until(lambda: result.exists(), "editor did not run after clear")
    assert "marker-1" not in result.read_text(), result.read_text()
    # Custom commands: a background shell command sees Herdr's variables; a pane command opens zoomed.
    focused = state()["focused"]
    os.write(master, b"\x02u")
    until(lambda: shell_result.exists() and shell_result.read_text(), "shell command did not run")
    assert shell_result.read_text() == f"{focused} {core.env['HC_SESSION']}", shell_result.read_text()
    os.write(master, b"\x02i")
    until(lambda: state()["focused"] != focused and state()["zoomed"] == state()["focused"], "pane command did not open a zoomed pane")
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: custom shell and pane commands, indexed workspace keys, notification target, numbered agent focus, direct pane resize, scrollback in $EDITOR, clear pane")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
