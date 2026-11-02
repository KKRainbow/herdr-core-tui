"""Shell EOF closes shared panes without another app, including hidden and final panes."""
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
core = Core("pane-exit", {"SHELL": "/bin/sh", "PS1": "EOF_READY> "})
process = master = slave = None
output = bytearray()
try:
    def command(name, args):
        return json.loads(subprocess.check_output([str(binary), "do", name, json.dumps(args)], env=core.env, timeout=10))

    def document():
        return core.rpc("doc.get", {"name": "herdr-app"})["value"]

    def drain():
        while select.select([master], [], [], 0)[0]:
            output.extend(os.read(master, 65536))

    def until(predicate, message, timeout=2):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            drain()
            if predicate():
                return
            time.sleep(.01)
        raise AssertionError(f"{message}: {document()}; terminals: {core.rpc('terminal.list', {'all': True})}; viewer: {state()}")

    def state():
        return json.loads(subprocess.check_output([str(binary), "--instance", "pane-exit", "view", "state"], env=core.env, timeout=5))

    def placed(terminal):
        return f'"terminal": "{terminal["address"]}"' in json.dumps(document())

    def ready(terminal):
        until(lambda: core.rpc("terminal.read", {"terminal": terminal["address"]})["text"].rstrip().endswith("EOF_READY>"), "shell prompt ready", timeout=10)

    source = command("space.new", {"folder": str(core.root), "argv": ["sh", "-i"]})
    root = source["terminal"]
    split = command("pane.split", {"id": source["pane"], "argv": ["sh", "-i"]})
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    original = termios.tcgetattr(slave)
    process = subprocess.Popen([str(binary), "--instance", "pane-exit"], env=core.env, stdin=slave, stdout=slave, stderr=slave)
    until(lambda: b"agents" in output, "workspace rendered", timeout=10)
    until(lambda: state()["metrics"]["attached"] == 2, "both shells attached", timeout=10)
    subprocess.run([str(binary), "--instance", "pane-exit", "view", "pane.focus", json.dumps({"id": split["pane"]})], env=core.env, capture_output=True, check=True, timeout=5)
    ready(split["terminal"])
    started = time.monotonic()
    os.write(master, b"\x04")
    until(lambda: core.rpc("terminal.get", {"terminal": split["terminal"]["address"]})["state"] == "finished", "Ctrl+D reached the shell")
    until(lambda: not placed(split["terminal"]), "exited split still placed")
    elapsed = time.monotonic() - started
    until(lambda: state()["view"]["focused"] == source["pane"] and len(state()["panes"]) == 1, "focus and layout follow closure")
    assert placed(root)
    assert core.rpc("terminal.get", {"terminal": root["address"]})["state"] == "live"

    busy = command("pane.split", {"id": source["pane"], "argv": ["sh", "-c", "while :; do printf 'BUSY\\n'; sleep .005; done"]})
    another = command("pane.split", {"id": source["pane"], "argv": ["sh", "-i"]})
    until(lambda: state()["metrics"]["attached"] == 3, "busy split and shell attached")
    subprocess.run([str(binary), "--instance", "pane-exit", "view", "pane.focus", json.dumps({"id": another["pane"]})], env=core.env, capture_output=True, check=True, timeout=5)
    ready(another["terminal"])
    os.write(master, b"\x04")
    until(lambda: not placed(another["terminal"]), "output from a neighbor delayed EOF closure")
    core.rpc("terminal.kill", {"terminal": busy["terminal"]["address"]})
    until(lambda: not placed(busy["terminal"]), "killed split still placed")

    hidden = command("tab.new", {"space": root["labels"]["herdr.space"], "argv": ["sh", "-i"], "env": {"PS1": "EOF_READY> "}})
    until(lambda: "EOF_READY>" in core.rpc("terminal.read", {"terminal": hidden["terminal"]["address"]})["text"], "hidden shell ready")
    core.rpc("terminal.keys", {"terminal": hidden["terminal"]["address"], "keys": ["ctrl+d"]})
    until(lambda: core.rpc("terminal.get", {"terminal": hidden["terminal"]["address"]})["state"] == "finished", "hidden shell exited")
    until(lambda: not placed(hidden["terminal"]), "exited hidden tab still placed")
    assert len(document()["spaces"][0]["tabs"]) == 1

    core.rpc("terminal.send", {"terminal": root["address"], "text": "kill -KILL $$"})
    def recovered():
        record = core.rpc("terminal.get", {"terminal": root["address"]})
        return record["state"] == "live" and record.get("pid") and record["pid"] != root["pid"]
    until(recovered, "kept shell recovered", timeout=10)
    until(lambda: state()["metrics"]["attached"] == 1, "recovered pane attached", timeout=10)
    assert placed(root), "a lost stream must not close its pane"

    ready(root)
    os.write(master, b"\x04")
    until(lambda: document()["spaces"] == [], "final shell left an empty workspace")
    until(lambda: state()["panes"] == [], "final pane still visible")
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    assert termios.tcgetattr(slave) == original

    stale = command("space.new", {"folder": str(core.root), "argv": ["sh", "-i"]})
    ready(stale["terminal"])
    core.rpc("terminal.keys", {"terminal": stale["terminal"]["address"], "keys": ["ctrl+d"]})
    until(lambda: core.rpc("terminal.get", {"terminal": stale["terminal"]["address"]})["state"] == "finished", "detached shell exited")
    assert placed(stale["terminal"])
    process = subprocess.Popen([str(binary), "--instance", "pane-exit"], env=core.env, stdin=slave, stdout=slave, stderr=slave)
    until(lambda: document()["spaces"] == [], "startup left a finished pane", timeout=10)
    until(lambda: state()["panes"] == [], "startup seeded a replacement workspace")
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    assert termios.tcgetattr(slave) == original
    print(f"PASS: Ctrl+D split closure in {elapsed:.3f}s, busy neighbor, focus fallback, hidden tab, kept recovery, final workspace, startup, host restoration")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
