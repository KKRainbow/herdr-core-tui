"""Core streams retain host color indices in both workspace and single-terminal views."""
import fcntl
import json
import os
import pathlib
import pty
import re
import select
import struct
import subprocess
import termios
import time

from fixture import Core

binary = pathlib.Path(os.environ.get(
    "TUI_TEST_BINARY", pathlib.Path(__file__).resolve().parents[1] / "target/debug/herder-tui"
)).resolve()
core = Core("colors")
core.env.pop("NO_COLOR", None)
process = master = slave = None
try:
    created = json.loads(subprocess.check_output([
        str(binary), "do", "space.new", json.dumps({
            "name": "Colors", "folder": str(core.root),
            "argv": ["sh", "-c", (
                "printf '\\033[31;44mPALETTE_RED\\033[0m "
                "\\033[38;5;202mPALETTE_202\\033[0m "
                "\\033[38;2;1;2;3mTRUECOLOR\\033[0m'; cat"
            )],
        }),
    ], env=core.env, timeout=15))
    address = created["terminal"]["address"]
    for target in ([], [address]):
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        process = subprocess.Popen(
            [str(binary), "--instance", "colors", *target], env=core.env,
            stdin=slave, stdout=slave, stderr=slave,
        )
        output = bytearray()

        def wait_for(marker):
            deadline = time.monotonic() + 10
            while marker not in output and time.monotonic() < deadline:
                if select.select([master], [], [], .05)[0]:
                    output.extend(os.read(master, 65536))
            assert marker in output, bytes(output[-4000:])

        wait_for(b"TRUECOLOR")
        for color in (b"38;5;1", b"48;5;4", b"38;5;202", b"38;2;1;2;3"):
            assert re.search(
                rb"\x1b\[(?:\d+;)*" + color + rb"(?:;\d+)*m", output
            ), (color, bytes(output))
        os.write(master, b"printf INPUT_OK\r")
        wait_for(b"INPUT_OK")
        os.write(master, b"\x11" if target else b"\x02q")
        assert process.wait(timeout=5) == 0
        assert termios.tcgetattr(slave) == original
        assert core.rpc("terminal.get", {"terminal": address})["state"] == "live"
        os.close(master)
        os.close(slave)
        master = slave = None
    print("PASS: core repaint preserves host palette and RGB in both views; input, detach and terminal restoration work")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
