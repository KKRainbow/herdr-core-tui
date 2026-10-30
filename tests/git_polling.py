"""Slow Git queries must run concurrently without delaying startup or detach."""
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

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herder-tui"
with tempfile.TemporaryDirectory(prefix="tui-slow-git-") as root:
    root = pathlib.Path(root)
    log = root / "queries"
    wrapper = root / "git"
    wrapper.write_text('''#!/usr/bin/python3
import fcntl, json, os, time
def record(event):
    with open(os.environ["GIT_QUERY_LOG"], "a") as log:
        fcntl.flock(log, fcntl.LOCK_EX)
        log.write(json.dumps([event, os.getpid()]) + "\\n")
record("start")
time.sleep(3)
record("end")
raise SystemExit(1)
''')
    wrapper.chmod(0o755)
    core = Core("git-polling", {"PATH": str(root) + os.pathsep + os.environ["PATH"], "GIT_QUERY_LOG": str(log)})
    process = master = slave = None
    try:
        spaces = []
        for index in range(6):
            folder = root / str(index)
            folder.mkdir()
            spaces.append({"id": f"s{index}", "name": f"Space {index}", "folder": str(folder), "tabs": []})
        current = core.rpc("doc.get", {"name": "herdr-app"})
        core.rpc("doc.set", {"name": "herdr-app", "version": current["version"], "value": {"spaces": spaces}})
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        started = time.monotonic()
        process = subprocess.Popen([str(binary), "--instance", "git-polling"], env=core.env, stdin=slave, stdout=slave, stderr=slave)
        output = b""
        while b"agents" not in output and time.monotonic() - started < 2:
            if select.select([master], [], [], .01)[0]:
                output += os.read(master, 65536)
        assert b"agents" in output, output[-2000:]
        startup = time.monotonic() - started

        def queries():
            return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

        # Let the first batch finish and the remaining folders start.
        end = time.monotonic() + 8
        while sum(event == "start" for event, _ in queries()) < 6 and time.monotonic() < end:
            if select.select([master], [], [], .02)[0]:
                os.read(master, 65536)
        events = queries()
        assert sum(event == "start" for event, _ in events) == 6, events
        active = peak = 0
        for event, _ in events:
            active += 1 if event == "start" else -1
            peak = max(peak, active)
        assert 2 <= peak <= 4 and active > 0, events
        started = time.monotonic()
        os.write(master, b"\x02q")
        assert process.wait(timeout=1) == 0
        detach = time.monotonic() - started
        assert detach < .5, detach
        assert termios.tcgetattr(slave) == original
        print(f"PASS: startup {startup:.3f}s, detach {detach:.3f}s during 3s Git queries, peak concurrency {peak}")
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
        if master is not None:
            os.close(master)
            os.close(slave)
        core.close()
