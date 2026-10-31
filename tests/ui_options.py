"""Herdr [ui] and [terminal] options: title, prompts, close confirmation, default shell, hidden sidebar."""
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
core = Core("ui-options")
config = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herder-tui" / "config.toml"
config.parent.mkdir(parents=True)
config.write_text("""
[ui]
window_title = "T:{workspace}"
prompt_new_workspace_name = true
prompt_new_tab_name = false
confirm_close = false
sidebar_start_collapsed = true
sidebar_collapsed_mode = "hidden"
hide_tab_bar_when_single_tab = true

[terminal]
default_shell = "/bin/sh"
""")
process = master = slave = None
try:
    first = json.loads(subprocess.check_output([str(binary), "do", "space.new", json.dumps({"name": "First"})], env=core.env, timeout=20))
    assert core.rpc("terminal.get", {"terminal": first["terminal"]["address"]})["argv"] == ["/bin/sh"]
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "ui"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    log = bytearray()

    def read_for(seconds=.1):
        output = b""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .02)[0]:
                output += os.read(master, 65536)
        log.extend(output)
        return output

    def wait_for(marker):
        end = time.monotonic() + 10
        while marker not in log and time.monotonic() < end:
            read_for(.05)
        assert marker in log, bytes(log[-2000:])
        return bytes(log)

    def spaces():
        return core.rpc("doc.get", {"name": "herdr-app"})["value"]["spaces"]

    def until(predicate, what):
        end = time.monotonic() + 10
        while time.monotonic() < end:
            read_for(.1)
            if predicate():
                return
        raise AssertionError((what, spaces()))

    wait_for(b"\x1b]0;T:First\x07")
    # Collapsed and hidden: no sidebar, and one tab hides the tab row.
    read_for(1)
    assert b"agents" not in log and b"spaces" not in log, bytes(log[-1500:])
    # A new workspace asks for its name first.
    os.write(master, b"\x02N")
    wait_for(b"new workspace")
    os.write(master, b"Second\r")
    until(lambda: [s["name"] for s in spaces()] == ["First", "Second"], "named workspace")
    wait_for(b"\x1b]0;T:Second\x07")
    # A new tab starts at once, without a name prompt.
    os.write(master, b"\x02c")
    until(lambda: len(spaces()[1]["tabs"]) == 2, "tab without prompt")
    # Closing a workspace needs no confirmation.
    os.write(master, b"\x02D")
    until(lambda: [s["name"] for s in spaces()] == ["First"], "close without confirmation")
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: window title, hidden collapsed sidebar, workspace name prompt, tab without prompt, close without confirmation, default shell")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
