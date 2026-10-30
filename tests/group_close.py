"""Group confirmation must reject new work before it closes any reviewed workspace."""
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
core = Core("group-close")
process = master = slave = None
try:
    repo = core.root / "repo"
    linked = core.root / "linked"
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    subprocess.run(["git", "-C", str(repo), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "initial", "--allow-empty"], check=True)
    subprocess.run(["git", "-C", str(repo), "worktree", "add", "-qb", "linked", str(linked)], check=True)

    def command(name, args):
        return json.loads(subprocess.check_output([str(binary), "do", name, json.dumps(args)], env=core.env, timeout=10))

    def document():
        return core.rpc("doc.get", {"name": "herdr-app"})["value"]

    terminals = []
    for folder in (repo, linked):
        terminals.append(command("space.new", {"folder": str(folder), "argv": ["sh", "-c", "sleep 180"]})["terminal"])
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "group-close"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def wait_for(marker):
        output = b""
        end = time.monotonic() + 10
        while marker not in output and time.monotonic() < end:
            if select.select([master], [], [], .05)[0]:
                output += os.read(master, 65536)
        assert marker in output, output[-2000:]

    wait_for(b"agents")
    os.write(master, b"\x02D")
    wait_for(b"Close worktree group?")
    # Add work to the last member through a second client while the dialog is open.
    node = document()["spaces"][-1]["tabs"][0]["root"]
    while "children" in node:
        node = node["children"][0]
    pane = node["id"]
    terminals.append(command("pane.split", {"id": pane, "argv": ["sh", "-c", "sleep 180"]})["terminal"])
    before = document()
    os.write(master, b"\r")
    wait_for(b"close target changed")
    assert document() == before
    for terminal in terminals:
        assert core.rpc("terminal.get", {"terminal": terminal["address"]})["state"] == "live"

    # A fresh review still closes the entire unchanged group after our own versioned writes.
    os.write(master, b"\x02D")
    wait_for(b"Close worktree group?")
    os.write(master, b"\r")
    end = time.monotonic() + 10
    while document()["spaces"] and time.monotonic() < end:
        if select.select([master], [], [], .05)[0]:
            os.read(master, 65536)
    assert document()["spaces"] == []
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: group close rejects new work before any close and accepts a fresh review")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
