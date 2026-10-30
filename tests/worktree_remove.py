"""Remove a dirty worktree through Herdr's two-step confirmation; new checkouts honor [worktrees] directory."""
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
core = Core("worktree-remove")
repo = core.root / "repo"
repo.mkdir()
trees = core.root / "trees"
config = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herder-tui"
config.mkdir(parents=True)
(config / "config.json").write_text(json.dumps({"worktrees": {"directory": str(trees)}, "keys": {"remove_worktree": "prefix+y"}}))
process = master = slave = None
try:
    def git(*args, cwd=repo):
        return subprocess.check_output(["git", "-C", str(cwd), *args]).decode().strip()

    def do(command, args):
        return json.loads(subprocess.check_output([str(binary), "do", command, json.dumps(args)], env=core.env, timeout=20))

    git("init", "-qb", "main")
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "initial", "--allow-empty")
    do("space.new", {"folder": str(repo), "argv": ["sh", "-c", "sleep 180"]})
    created = do("worktree.new", {"repository": str(repo), "branch": "feature/dirty"})
    checkout = trees / "repo" / "feature-dirty"
    assert created["ok"] and checkout.is_dir(), created
    space = created["workspace"]["terminal"]["labels"]["herdr.space"]
    (checkout / "untracked").write_text("work")
    review = do("worktree.remove", {"repository": str(repo), "path": str(checkout), "id": space})
    assert review["risks"] == ["dirty or untracked files"], review
    refused = subprocess.run([str(binary), "do", "worktree.remove", json.dumps({"repository": str(repo), "path": str(checkout), "id": space, "confirm": True})], env=core.env, capture_output=True, timeout=20)
    assert refused.returncode != 0 and b"confirm with force" in refused.stdout + refused.stderr, refused
    assert checkout.is_dir()

    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "worktree-remove"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def read_for(seconds=.05):
        output = b""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .01)[0]:
                output += os.read(master, 65536)
        return output

    def wait_for(marker):
        output = b""
        end = time.monotonic() + 15
        while marker not in output and time.monotonic() < end:
            output += read_for()
        assert marker in output, output[-2000:]
        return output

    def view(command, args):
        result = subprocess.run([str(binary), "--instance", "worktree-remove", "view", command, json.dumps(args)], env=core.env, capture_output=True, timeout=5)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return json.loads(result.stdout)

    wait_for(b"feature")
    view("space.focus", {"id": space})
    read_for(.5)
    os.write(master, b"\x02y")
    wait_for(b"delete worktree checkout")
    os.write(master, b"\r")
    wait_for(b"permanently deleted")
    assert checkout.is_dir()
    os.write(master, b"\r")
    end = time.monotonic() + 15
    while checkout.exists() and time.monotonic() < end:
        read_for()
    assert not checkout.exists()
    assert str(checkout) not in git("worktree", "list")
    assert git("rev-parse", "--verify", "feature/dirty")
    spaces = core.rpc("doc.get", {"name": "herdr-app"})["value"]["spaces"]
    assert all(s["id"] != space for s in spaces), spaces
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: configured worktree root, dirty removal refused without force, Herdr's delete-anyway confirmation removes checkout and space, branch kept")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    core.close()
