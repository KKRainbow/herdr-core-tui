"""Linux integration test: isolate core, attach via a host PTY, and restore it."""
import fcntl
import json
import os
import pathlib
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herder-tui"
with tempfile.TemporaryDirectory(prefix="herder-tui-test-") as root:
    env = dict({k:v for k,v in os.environ.items() if k not in ("WAYLAND_DISPLAY","DISPLAY")}, HC_SESSION="tui-smoke", XDG_RUNTIME_DIR=root + "/runtime",
               XDG_CONFIG_HOME=root + "/config", XDG_STATE_HOME=root + "/state",
               HERDR_APP_HOME=root + "/app", TERM="xterm-256color")
    pathlib.Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
    hc = env.get("HC_BINARY", "hc")
    def core(*args):
        return subprocess.check_output([hc, *args], env=env, timeout=15)
    try:
        terminal = json.loads(core("run", "--json", "--bg", "--keep", "sh", "-c", "printf TUI_FIXTURE; cat"))
        address = terminal["address"]
        for target, expected in [(address, 0), (address, 0), ("missing-terminal", 1)]:
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
            original = termios.tcgetattr(slave)
            process = subprocess.Popen([str(binary), "--session", env["HC_SESSION"], target],
                                       env=env, stdin=slave, stdout=slave, stderr=slave)
            output = b""
            sent = False
            deadline = time.monotonic() + 15
            try:
                while process.poll() is None and time.monotonic() < deadline:
                    if select.select([master], [], [], .1)[0]:
                        output += os.read(master, 65536)
                    if b"TUI_FIXTURE" in output and not sent:
                        os.write(master, b"\x11")
                        sent = True
                assert process.poll() is not None, output[-4000:]
                assert process.wait(timeout=3) == expected, output
                assert termios.tcgetattr(slave) == original, "host terminal settings changed"
                if expected == 0:
                    assert b"TUI_FIXTURE" in output, output
                    assert b"\x1b[?1049l" in output, "alternate screen was not restored"
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
                os.close(slave)
        assert b'live' in core("get", address), "detach ended the shared terminal"
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        original = termios.tcgetattr(slave)
        process = subprocess.Popen([str(binary), "--session", env["HC_SESSION"], address], env=env, stdin=slave, stdout=slave, stderr=slave)
        def await_output(marker):
            output = b""
            deadline = time.monotonic() + 15
            while marker not in output and time.monotonic() < deadline:
                if select.select([master], [], [], .1)[0]:
                    output += os.read(master, 65536)
            assert marker in output, output[-2000:]
        try:
            await_output(b"TUI_FIXTURE")
            core("server", "restart")
            time.sleep(2)
            core("send", address, "printf RECOVERY_OK")
            await_output(b"RECOVERY_OK")
            os.write(master, b"\x11")
            assert process.wait(timeout=5) == 0
            assert termios.tcgetattr(slave) == original
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)
        core("kill", address)
        print("PASS: stream repaint, detach, live process, normal/error host restoration")
    finally:
        core("server", "stop")
