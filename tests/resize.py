"""Host resizes while panes stream output must not crash or lose the host terminal."""
import fcntl
import os
import pathlib
import pty
import random
import select
import signal
import struct
import subprocess
import termios
import time

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herdr-tui"
core = Core("resize")
process = master = slave = None
try:
    master, slave = pty.openpty()

    def size(rows, cols):
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    size(40, 178)
    original = termios.tcgetattr(slave)
    process = subprocess.Popen([str(binary), "--instance", "resize"], env=core.env, stdin=slave, stdout=slave, stderr=subprocess.PIPE)

    def drain(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .01)[0]:
                os.read(master, 1 << 16)

    drain(2)
    os.write(master, b"\x02N")
    drain(1)
    for _ in range(5):
        os.write(master, b"\x02v")
        drain(.5)
    for _ in range(6):
        os.write(master, b"while :; do date; done\r\x02\t")
        drain(.3)
    # Each resize makes every visible stream resize, which leaves the host time to resize again.
    random.seed(1)
    for _ in range(400):
        size(random.randint(20, 45), random.randint(100, 180))
        process.send_signal(signal.SIGWINCH)
        drain(random.random() * .03)
        assert process.poll() is None, process.stderr.read().decode()[-2000:]
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    assert termios.tcgetattr(slave) == original
    print("PASS: 400 host resizes during streaming output in six panes")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
