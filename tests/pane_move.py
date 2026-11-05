"""Move the same live pane between tabs and workspaces through its context menu."""
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

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herdr-tui"
core = Core("pane-move")
process = master = slave = None
try:
    def command(name, args):
        return json.loads(subprocess.check_output([str(binary), "do", name, json.dumps(args)], env=core.env, timeout=10))

    def document():
        return core.rpc("doc.get", {"name": "herdr-app"})["value"]

    source = command("space.new", {"name": "Source", "folder": str(core.root), "argv": ["sh", "-c", "sleep 180"]})
    pane = source["pane"]
    terminal = source["terminal"]
    space = terminal["labels"]["herdr.space"]
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "pane-move"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def read_for(seconds=.05):
        output = b""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .01)[0]:
                output += os.read(master, 65536)
        return output

    def wait_for(marker):
        output = b""
        end = time.monotonic() + 10
        while marker not in output and time.monotonic() < end:
            output += read_for()
        assert marker in output, output[-2000:]
        return output

    def click(x, y, button=0):
        read_for()
        os.write(master, f"\x1b[<{button};{x};{y}M\x1b[<{button};{x};{y}m".encode())

    def open_picker(x=60):
        click(x, 10, 2)
        wait_for(b"Move pane...")
        os.write(master, b"jjj\r")
        return wait_for(b"move pane to tab")

    def until(predicate):
        end = time.monotonic() + 10
        while time.monotonic() < end:
            read_for()
            if predicate():
                return
        raise AssertionError(document())

    def location(node):
        if node.get("id") == pane:
            return True
        return any(location(child) for key in ("spaces", "tabs", "children") for child in node.get(key, [])) or ("root" in node and location(node["root"]))

    def moved_to(tab):
        return any(t["id"] == tab and location(t) for s in document()["spaces"] for t in s["tabs"])

    def verify(tab, workspace):
        until(lambda: moved_to(tab))
        record = core.rpc("terminal.get", {"terminal": terminal["address"]})
        assert record["state"] == "live" and record["pid"] == terminal["pid"], record
        assert record["labels"]["herdr.space"] == workspace and record["labels"]["herdr.tab"] == tab, record
        state = json.loads(subprocess.check_output([str(binary), "--instance", "pane-move", "view", "state"], env=core.env, timeout=5))
        assert state["view"]["focused"] == pane and state["view"]["active_tab"] == tab, state

    wait_for(b"agents")
    before = document()
    assert b"no other tabs" in open_picker()
    os.write(master, b"\x1b")
    read_for()
    assert document() == before
    same = command("tab.new", {"space": space, "name": "Same space", "argv": ["sh", "-c", "sleep 180"]})["tab"]
    other = command("space.new", {"name": "Other space", "folder": str(core.root), "argv": ["sh", "-c", "sleep 180"]})
    read_for(.2)
    before = document()
    output = open_picker()
    assert b"Same space" in output and b"Other space" in output, output
    os.write(master, b"\x1b")
    read_for()
    assert document() == before
    open_picker()
    os.write(master, b"\r")
    verify(same, space)
    open_picker(40)
    click(30, 11)
    click(44, 16)
    verify(other["tab"], other["terminal"]["labels"]["herdr.space"])
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: empty and cancelled chooser, keyboard tab move, mouse workspace move, live process and labels preserved, focus follows pane")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
