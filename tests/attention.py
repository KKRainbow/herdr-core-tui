"""An unwatched finish stays unseen across a viewer restart and clears when viewed."""
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
core = Core("attention")
instance = "attention"
state = pathlib.Path(core.env["XDG_STATE_HOME"]) / "herdr-tui" / ("attention-" + f"{core.env['HC_SESSION']}:{instance}".encode().hex() + ".json")
report = "printf '\\033]7501;state=working:app=codex\\033\\\\'; sleep 3; printf '\\033]7501;state=idle:app=codex\\033\\\\'; sleep 180"
process = master = slave = None
try:
    def do(command, args):
        return json.loads(subprocess.check_output([str(binary), "do", command, json.dumps(args)], env=core.env, timeout=20))

    def view(command, args):
        # A shared write from another client briefly leaves the viewer refreshing.
        end = time.monotonic() + 10
        while True:
            result = subprocess.run([str(binary), "--instance", instance, "view", command, json.dumps(args)], env=core.env, capture_output=True, timeout=5)
            if result.returncode == 0 or time.monotonic() > end:
                break
            drain(.2)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return json.loads(result.stdout)

    def start():
        global process, master, slave
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        process = subprocess.Popen([str(binary), "--instance", instance], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def drain(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .05)[0]:
                os.read(master, 65536)

    def stop():
        global master, slave
        os.write(master, b"\x02q")
        assert process.wait(timeout=5) == 0
        os.close(master)
        os.close(slave)
        master = slave = None

    def seen(address):
        entries = json.loads(state.read_text())["entries"] if state.exists() else []
        return next((e["seen"] for e in entries if e["identity"]["local"] == address and e["word"] == "idle"), None)

    def wait_seen(address, expected):
        end = time.monotonic() + 15
        while seen(address) is not expected and time.monotonic() < end:
            drain(.2)
        assert seen(address) is expected, state.read_text() if state.exists() else "no state"

    shell = do("space.new", {"name": "Attention", "argv": ["sh", "-c", "sleep 180"]})
    space = shell["terminal"]["labels"]["herdr.space"]
    start()
    drain(1)
    agent = do("tab.new", {"space": space, "argv": ["sh", "-c", report]})
    address, pane = agent["terminal"]["address"], agent["pane"]
    view("tab.focus", {"id": shell["tab"]})
    wait_seen(address, False)
    stop()
    start()
    drain(5)
    # The restored finish is still news after the restart.
    assert seen(address) is False, state.read_text()
    view("pane.focus", {"id": pane})
    wait_seen(address, True)
    stop()
    print("PASS: unwatched finish saved as unseen, restored across restart, cleared when viewed")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
