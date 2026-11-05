"""Herdr's [ui.toast] delivery: a card inside the TUI, OSC 9 to the terminal, the desktop, and the delay recheck."""
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
finish = "printf '\\033]7501;state=working:app=codex\\033\\\\'; sleep 2; printf '\\033]7501;state=idle:app=codex\\033\\\\'; sleep 180"
block = "printf '\\033]7501;state=working:app=codex\\033\\\\'; sleep 2; printf '\\033]7501;state=blocked:app=codex\\033\\\\'; sleep 180"


def run(name, toast, env, script, steps):
    core = Core(name, env)
    config = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herdr-tui" / "config.toml"
    config.parent.mkdir(parents=True)
    config.write_text("[ui.sound]\nenabled = false\n\n[ui.toast]\n" + toast)
    process = master = slave = None
    log = bytearray()
    try:
        def do(command, args):
            return json.loads(subprocess.check_output([str(binary), "do", command, json.dumps(args)], env=core.env, timeout=20))

        shell = do("space.new", {"name": "Notify", "argv": ["sh", "-c", "sleep 180"]})
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        process = subprocess.Popen([str(binary), "--instance", name], env=core.env, stdin=slave, stdout=slave, stderr=slave)

        def read_for(seconds):
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if select.select([master], [], [], .05)[0]:
                    log.extend(os.read(master, 65536))

        def wait_for(marker, seconds=12):
            end = time.monotonic() + seconds
            while marker not in log and time.monotonic() < end:
                read_for(.1)
            assert marker in log, (marker, bytes(log[-1500:]))

        def view(command, args):
            out = subprocess.run([str(binary), "--instance", name, "view", command, json.dumps(args)], env=core.env, capture_output=True, timeout=5)
            return json.loads(out.stdout)

        read_for(1.5)
        agent = do("tab.new", {"space": shell["space"], "name": "work", "argv": ["sh", "-c", script]})
        read_for(.5)
        view("tab.focus", {"id": shell["tab"]})
        steps.send = lambda data: os.write(master, data)
        steps(core, wait_for, read_for, view, agent)
        os.write(master, b"\x02q")
        assert process.wait(timeout=5) == 0
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
        if master is not None:
            os.close(master)
            os.close(slave)
        core.close()


def card_steps(core, wait_for, read_for, view, agent):
    wait_for(b"codex finished")
    assert view("state", {})["view"]["focused"] != agent["pane"]
    # prefix+o opens the card's agent.
    card_steps.send(b"\x02o")
    end = time.monotonic() + 5
    while view("state", {})["view"]["focused"] != agent["pane"] and time.monotonic() < end:
        read_for(.2)
    assert view("state", {})["view"]["focused"] == agent["pane"]


run("notify-card", 'delivery = "herdr"\n', None, finish, card_steps)

notified = pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / f"tui-notify-{os.getpid()}.txt"
fake = pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / f"tui-notify-send-{os.getpid()}"
fake.write_text(f"#!/bin/sh\nprintf '%s\\n' \"$@\" >> {notified}\n")
fake.chmod(0o700)
try:
    def system_steps(core, wait_for, read_for, view, agent):
        end = time.monotonic() + 12
        while not (notified.exists() and "needs attention" in notified.read_text()) and time.monotonic() < end:
            read_for(.2)
        text = notified.read_text()
        assert "codex needs attention" in text and "Notify · 1 · work" in text, text

    run("notify-system", 'delivery = "system"\n', {"HERDR_TUI_NOTIFY_SEND": str(fake), "DISPLAY": ":99"}, block, system_steps)

    notified.unlink()

    def answered_steps(core, wait_for, read_for, view, agent):
        # The request is answered before the delay ends, so nothing is delivered.
        read_for(3)
        subprocess.run(["hc", "send", agent["terminal"]["address"], "x"], env=core.env, capture_output=True, timeout=10)
        core.rpc("terminal.kill", {"terminal": agent["terminal"]["address"]})
        read_for(5)
        assert not notified.exists(), notified.read_text()

    run("notify-delay", 'delivery = "system"\ndelay_seconds = 4\n', {"HERDR_TUI_NOTIFY_SEND": str(fake), "DISPLAY": ":99"}, block, answered_steps)
finally:
    fake.unlink(missing_ok=True)
    notified.unlink(missing_ok=True)


def terminal_steps(core, wait_for, read_for, view, agent):
    wait_for(b"\x1b]9;codex finished: Notify")


run("notify-terminal", 'delivery = "terminal"\n', {"TERM_PROGRAM": "WezTerm"}, finish, terminal_steps)
print("PASS: card inside the TUI, desktop notification with Herdr's context, delay recheck drops answered requests, OSC 9 to the terminal")
