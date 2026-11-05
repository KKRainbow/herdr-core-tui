"""Herdr's mobile layout at phone width: header, agent summary, switcher by mouse and keyboard."""
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
core = Core("mobile")
block = "printf '\\033]7501;state=blocked:app=codex\\033\\\\'; sleep 180"
process = master = slave = None
log = bytearray()
try:
    def do(command, args):
        return json.loads(subprocess.check_output([str(binary), "do", command, json.dumps(args)], env=core.env, timeout=20))

    first = do("space.new", {"name": "Alpha", "argv": ["sh", "-c", "sleep 180"]})
    second = do("space.new", {"name": "Beta", "argv": ["sh", "-c", block]})
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 50, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "mobile"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def read_for(seconds=.1):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .02)[0]:
                log.extend(os.read(master, 65536))

    def wait_for(marker):
        end = time.monotonic() + 10
        while marker not in log and time.monotonic() < end:
            read_for(.05)
        assert marker in log, (marker, bytes(log[-2000:]))

    def state():
        out = subprocess.run([str(binary), "--instance", "mobile", "view", "state", "{}"], env=core.env, capture_output=True, timeout=5)
        return json.loads(out.stdout)["view"]

    def click(x, y):
        os.write(master, f"\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m".encode())

    # No sidebar: a header with the space, its tab, the agent summary, and switch.
    wait_for(b"switch")
    wait_for(b"tab 1")
    wait_for(b"1 blocked")
    assert b"agents" not in log, bytes(log[-1500:])
    subprocess.run([str(binary), "--instance", "mobile", "view", "space.focus", json.dumps({"id": first["space"]})], env=core.env, capture_output=True, timeout=5)
    read_for(.5)
    # The switch button opens the switcher; arrows and Enter pick a space.
    click(46, 1)
    wait_for(b"+ new workspace")
    wait_for(b"close")
    os.write(master, b"\x1b[B")
    read_for(.3)
    os.write(master, b"\r")
    end = time.monotonic() + 5
    while state()["active_space"] != second["space"] and time.monotonic() < end:
        read_for(.1)
    assert state()["active_space"] == second["space"], state()
    # A switcher row opens its agent by mouse: the agents section comes first.
    click(46, 1)
    log.clear()
    wait_for(b"+ new workspace")
    subprocess.run([str(binary), "--instance", "mobile", "view", "space.focus", json.dumps({"id": first["space"]})], env=core.env, capture_output=True, timeout=5)
    read_for(.5)
    click(10, 5)
    end = time.monotonic() + 5
    while state()["focused"] != second["pane"] and time.monotonic() < end:
        read_for(.1)
    assert state()["focused"] == second["pane"], state()
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: mobile header and agent summary, switcher by switch button, keyboard space choice, agent row by mouse")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
