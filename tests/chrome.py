"""Exercise Herdr's sidebar, menus, tab bar, and pane context menu with a mouse."""
import fcntl
import json
import os
import pathlib
import pty
import select
import signal
import struct
import subprocess
import termios
import time

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herder-tui"
core = Core("chrome")
process = master = slave = None
try:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    original = termios.tcgetattr(slave)
    process = subprocess.Popen([str(binary), "--instance", "chrome"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def read_for(seconds=.2):
        output = b""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .02)[0]:
                output += os.read(master, 65536)
        return output

    def wait_for(marker):
        output = b""
        end = time.monotonic() + 10
        while marker not in output and time.monotonic() < end:
            output += read_for(.05)
        assert marker in output, output[-2000:]
        return output

    def click(x, y, button=0):
        read_for(.05)
        os.write(master, f"\x1b[<{button};{x};{y}M\x1b[<{button};{x};{y}m".encode())

    def document():
        return core.rpc("doc.get", {"name": "herdr-app"})["value"]

    def until(predicate):
        end = time.monotonic() + 10
        while time.monotonic() < end:
            read_for(.05)
            if predicate():
                return
        raise AssertionError("shared layout did not reflect mouse action")

    def count(node):
        return sum(count(c) for c in node["children"]) if "children" in node else 1

    # A 26-column sidebar: spaces above, agents below, footer on row 12.
    output = wait_for(b"agents")
    assert b"spaces" in output and b"new" in output and b"menu" in output, output[-2000:]
    click(22, 12)
    wait_for(b"keybinds")
    os.write(master, b"\x1b")
    read_for()
    click(2, 12)
    until(lambda: len((document() or {}).get("spaces", [])) == 1)
    space = document()["spaces"][0]
    assert space["name"] == "" and space["tabs"][0]["name"] == "", space

    # The tab bar starts after the sidebar: tab 1 is eight columns, then a gap and "+".
    click(37, 1)
    wait_for(b"new tab")
    os.write(master, b"Mouse tab\r")
    until(lambda: [t["name"] for t in document()["spaces"][0]["tabs"]] == ["", "Mouse tab"])

    # Right-click a pane, then pick "Split right" (second item) from Herdr's pane menu.
    click(60, 10, 2)
    wait_for(b"Split right")
    os.write(master, b"j\r")
    until(lambda: count(document()["spaces"][0]["tabs"][1]["root"]) == 2)
    os.write(master, b"\x02-")
    until(lambda: count(document()["spaces"][0]["tabs"][1]["root"]) == 3)

    tab = document()["spaces"][0]["tabs"][1]["id"]
    subprocess.run([str(binary), "do", "tab.rename", json.dumps({"id": tab, "name": "Long tab title " * 12})], env=core.env, stdout=subprocess.DEVNULL, check=True)
    wait_for(b" > ")

    # Right-click the space row for its menu; Rename opens Herdr's prompt.
    click(5, 3, 2)
    wait_for(b"Rename")
    os.write(master, b"\r")
    wait_for(b"rename workspace")
    os.write(master, b"\x03Mouse space\r")
    until(lambda: document()["spaces"][0]["name"] == "Mouse space")

    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 60, 0, 0))
    os.kill(process.pid, signal.SIGWINCH)
    output = read_for(.5)
    assert b"new" in output and b"menu" in output, output[-2000:]
    os.write(master, b"\x02b")
    wait_for("»".encode())
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    assert termios.tcgetattr(slave) == original
    print("PASS: spaces/agents sidebar, new and menu buttons, + tab prompt, pane and space context menus, tab overflow, narrow host, collapse, detach")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
