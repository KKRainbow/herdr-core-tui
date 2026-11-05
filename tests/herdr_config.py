"""Without its own file, the TUI reads Herdr's config.toml; the first settings change copies it."""
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
import tomllib

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herdr-tui"
core = Core("herdr-config")
herdr = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herdr" / "config.toml"
herdr.parent.mkdir(parents=True)
original = '# from herdr\n[keys]\nsettings = "prefix+y"\n\n[ui.sound]\nenabled = true\n'
herdr.write_text(original)
own = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herdr-tui" / "config.toml"
process = master = slave = None
try:
    subprocess.check_output([str(binary), "do", "space.new", json.dumps({"argv": ["sh", "-c", "sleep 180"]})], env=core.env, timeout=20)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "herdr-config"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def wait_for(marker):
        output = b""
        end = time.monotonic() + 10
        while marker not in output and time.monotonic() < end:
            if select.select([master], [], [], .05)[0]:
                output += os.read(master, 65536)
        assert marker in output, output[-2000:]

    wait_for(b"agents")
    os.write(master, b"\x02y")
    wait_for(b"settings")
    os.write(master, b"\t\t\x1b[B\r")
    end = time.monotonic() + 5
    while not own.exists() and time.monotonic() < end:
        time.sleep(.1)
    copied = tomllib.loads(own.read_text())
    assert copied["keys"]["settings"] == "prefix+y" and copied["ui"]["sound"]["enabled"] is False, copied
    assert own.read_text().startswith("# from herdr")
    assert herdr.read_text() == original
    os.write(master, b"\x1b")
    time.sleep(.3)
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: Herdr's config.toml applies without an own file; first settings change copies it and leaves Herdr's file alone")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
