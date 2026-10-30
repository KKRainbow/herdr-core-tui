"""Attach to an existing fake terminal in a separately isolated app fixture."""
import fcntl
import os
import pathlib
import pty
import select
import struct
import subprocess
import termios
import time
root = os.environ["TUI_APP_TEST_ROOT"]
assert root.startswith("/tmp/")
address = os.environ["TUI_APP_TEST_TERMINAL"]
env = dict(os.environ, HERDR_APP_HOME=root + "/app", XDG_RUNTIME_DIR=root + "/runtime",
           XDG_STATE_HOME=root + "/state", XDG_CONFIG_HOME=root + "/config", TERM="xterm-256color")
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
original = termios.tcgetattr(slave)
binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herder-tui"
process = subprocess.Popen([str(binary), "--session", env["HC_SESSION"], address],env=env,stdin=slave,stdout=slave,stderr=slave)
try:
    output=b""
    deadline=time.monotonic()+10
    while b"APP_TUI_SHARED" not in output and time.monotonic()<deadline:
        if select.select([master],[],[],.1)[0]: output+=os.read(master,65536)
    assert b"APP_TUI_SHARED" in output, output[-1000:]
    os.write(master,b"TUI_INPUT\r")
    time.sleep(.3)
    assert b"TUI_INPUT" in subprocess.check_output(["hc","read",address],env=env)
    os.write(master,b"\x11")
    assert process.wait(timeout=5)==0
    assert termios.tcgetattr(slave)==original
    assert address.encode() in subprocess.check_output(["herdr-app","ls"],env=env)
    assert b'live' in subprocess.check_output(["hc","get",address],env=env)
    print("PASS: app-created terminal repaint, TUI input, detach, app retains terminal")
finally:
    if process.poll() is None: process.kill(); process.wait()
    os.close(master)
    os.close(slave)
