"""A new session opens a workspace in the starting folder, as Herdr does; an emptied one stays empty."""
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
core = Core("startup")
folder = core.root / "project"
folder.mkdir()
process = master = slave = None
try:
    def run(instance):
        global process, master, slave
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
        process = subprocess.Popen([str(binary), "--instance", instance], env=core.env, cwd=folder, stdin=slave, stdout=slave, stderr=slave)
        output = b""
        end = time.monotonic() + 10
        while b"agents" not in output and time.monotonic() < end:
            if select.select([master], [], [], .05)[0]:
                output += os.read(master, 65536)
        time.sleep(.5)
        os.write(master, b"\x02q")
        assert process.wait(timeout=5) == 0
        os.close(master)
        os.close(slave)
        master = slave = None
        return core.rpc("doc.get", {"name": "herdr-app"})

    document = run("first")
    spaces = document["value"]["spaces"]
    assert len(spaces) == 1 and spaces[0]["folder"] == str(folder) and spaces[0]["name"] == "", spaces
    terminal = spaces[0]["tabs"][0]["root"]["children"][0]["content"]["terminal"]
    assert core.rpc("terminal.get", {"terminal": terminal})["cwd"] == str(folder)
    # A second viewer of the same session adds nothing.
    assert len(run("second")["value"]["spaces"]) == 1
    subprocess.run([str(binary), "do", "space.close", json.dumps({"id": spaces[0]["id"], "confirm": True})], env=core.env, capture_output=True, check=True, timeout=20)
    assert run("third")["value"]["spaces"] == []
    print("PASS: new session opens a workspace in the starting folder, reattach adds nothing, emptied session stays empty")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
