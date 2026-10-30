"""Use hc session defaults and reach terminals without agent reports from Go to."""
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
for session, inherited, flags in (
    ("default", None, []),
    ("discovery", " discovery ", []),
    ("default", "wrong-session", ["--session", "default"]),
):
    core = Core(session)
    process = master = slave = None
    try:
        placed = core.rpc("terminal.run", {"argv": ["sh", "-c", "printf PLACED_TERMINAL; cat"], "no_parent": True})
        loose = core.rpc("terminal.run", {"argv": ["sh", "-c", "printf UNPLACED_TERMINAL; cat"], "no_parent": True})
        assert placed["reported"] is False and loose["reported"] is False
        document = {"spaces": [{"id": "space", "name": "Shared space", "tabs": [{"id": "tab", "name": "Shared tab", "root": {"id": "tab", "type": "row", "children": [{"id": "pane", "type": "block", "content": {"terminal": placed["address"]}}]}}]}]}
        current = core.rpc("doc.get", {"name": "herdr-app"})
        core.rpc("doc.set", {"name": "herdr-app", "version": current["version"], "value": document})
        before = core.rpc("doc.get", {"name": "herdr-app"})
        env = dict(core.env)
        if inherited is None:
            env.pop("HC_SESSION", None)
        else:
            env["HC_SESSION"] = inherited
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        process = subprocess.Popen([str(binary), *flags, "--instance", "discovery"], env=env, stdin=slave, stdout=slave, stderr=slave)

        def wait_for(marker):
            output = b""
            end = time.monotonic() + 10
            while marker not in output and time.monotonic() < end:
                if select.select([master], [], [], .05)[0]:
                    output += os.read(master, 65536)
            assert marker in output, output[-2000:]
            return output

        def view(command, args=None):
            return json.loads(subprocess.check_output([str(binary), *flags, "--instance", "discovery", "view", command, json.dumps(args or {})], env=env, timeout=5))

        output = wait_for(b"PLACED_TERMINAL")
        assert b"spaces" in output and b"agents" in output, output[-2000:]
        assert view("state")["session"] == session
        assert view("state")["view"]["focused"] == "pane"
        os.write(master, b"\x02g")
        wait_for(b"no pane")
        os.write(master, b"/" + loose["address"].encode() + b"\r")
        wait_for(b"UNPLACED_TERMINAL")
        assert view("state")["panes"][0]["terminal"] == loose["address"]
        assert core.rpc("doc.get", {"name": "herdr-app"}) == before
        view("terminal.focus", {"terminal": placed["address"]})
        assert view("state")["view"]["focused"] == "pane"
        view("quit")
        assert process.wait(timeout=5) == 0
        assert termios.tcgetattr(slave) == original
        assert core.rpc("terminal.get", {"terminal": loose["address"]})["state"] == "live"
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
        if master is not None:
            os.close(master)
            os.close(slave)
        core.close()
print("PASS: default/inherited/explicit sessions, shared panes, ordinary unplaced terminals, private preview, detach")
