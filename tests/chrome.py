"""Exercise persistent workspace, menu, tab, and split buttons with a mouse."""
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

    def click(x, y):
        read_for(.05)
        os.write(master, f"\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m".encode())

    def document():
        return core.rpc("doc.get", {"name": "herdr-app"})["value"]

    def until(predicate):
        end = time.monotonic() + 10
        while time.monotonic() < end:
            read_for(.05)
            if predicate():
                return
        raise AssertionError("shared layout did not reflect mouse action")

    output = wait_for(b"Ctrl+Q")
    assert b"new" in output and b"menu" in output and b"spaces" in output, output[-2000:]
    click(21, 12)
    wait_for(b"Actions")
    os.write(master, b"\x1b")
    read_for()
    click(2, 12)
    wait_for(b"space.new")
    os.write(master, ("Mouse workspace\t" + str(core.root) + "\tprintf CHROME_FIRST; cat\r").encode())
    until(lambda: len((document() or {}).get("spaces", [])) == 1)
    click(36, 1)
    wait_for(b"tab.new")
    os.write(master, b"Mouse tab\tprintf CHROME_TAB; cat\r")
    until(lambda: len(document()["spaces"][0]["tabs"]) == 2)
    click(97, 2)
    wait_for(b"pane.split")
    os.write(master, b"\tprintf CHROME_DOWN; cat\r")

    def count(node):
        return sum(count(c) for c in node["children"]) if "children" in node else 1

    until(lambda: count(document()["spaces"][0]["tabs"][1]["root"]) == 2)
    click(94, 13)
    wait_for(b"pane.split")
    os.write(master, b"\tprintf CHROME_RIGHT; cat\r")
    until(lambda: count(document()["spaces"][0]["tabs"][1]["root"]) == 3)

    tab = document()["spaces"][0]["tabs"][1]["id"]
    subprocess.run([str(binary), "do", "tab.rename", json.dumps({"id": tab, "name": "Long tab title " * 12})], env=core.env, stdout=subprocess.DEVNULL, check=True)
    read_for()
    click(99, 1)
    wait_for(b"tab.new")
    os.write(master, b"\x1b")
    read_for()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 40, 0, 0))
    os.kill(process.pid, signal.SIGWINCH)
    output = read_for(.5)
    assert b"new" in output and b"menu" in output, output[-2000:]
    click(10, 6)
    wait_for(b"Actions")
    os.write(master, b"\x1b")
    read_for()
    click(2, 6)
    wait_for(b"space.new")
    os.write(master, b"\x1b")
    read_for()
    os.write(master, b"\x11")
    assert process.wait(timeout=5) == 0
    assert termios.tcgetattr(slave) == original
    print("PASS: empty-state New/Menu, mouse workspace/tab creation, both split buttons, tab overflow, narrow-host controls, host restore")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
