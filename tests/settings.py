"""Herdr's settings overlay: theme preview and apply, indicators, sound, written to the config file."""
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
core = Core("settings")
config = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herder-tui" / "config.json"
config.parent.mkdir(parents=True)
config.write_text(json.dumps({"keys": {"next_agent": "alt+a"}}))
process = master = slave = None
try:
    subprocess.check_output([str(binary), "do", "space.new", json.dumps({"argv": ["sh", "-c", "sleep 180"]})], env=core.env, timeout=20)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "settings"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def read_for(seconds=.1):
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

    def saved():
        return json.loads(config.read_text())

    wait_for(b"agents")
    os.write(master, b"\x02s")
    wait_for(b"settings")
    # Previewing a theme and leaving with Esc writes nothing.
    os.write(master, b"\x1b[B")
    read_for(.3)
    os.write(master, b"\x1b")
    read_for(.3)
    assert "theme" not in saved(), saved()
    # Indicators and sound apply in place and keep other settings.
    os.write(master, b"\x02s")
    wait_for(b"settings")
    os.write(master, b"\t\x1b[B\r")
    end = time.monotonic() + 5
    while saved().get("status_indicators") != "symbols" and time.monotonic() < end:
        read_for()
    assert saved()["status_indicators"] == "symbols", saved()
    os.write(master, b"\t\x1b[B\r")
    end = time.monotonic() + 5
    while saved().get("sound", {}).get("enabled") is not False and time.monotonic() < end:
        read_for()
    assert saved()["sound"]["enabled"] is False, saved()
    # Back to themes: the third entry applies and closes settings.
    os.write(master, b"\t\x1b[B\x1b[B\x1b[B\r")
    end = time.monotonic() + 5
    while saved().get("theme") != "tokyo-night" and time.monotonic() < end:
        read_for()
    assert saved()["theme"] == "tokyo-night" and saved()["keys"] == {"next_agent": "alt+a"}, saved()
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: settings preview without saving, indicators and sound in place, theme applies and closes, other config kept")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
