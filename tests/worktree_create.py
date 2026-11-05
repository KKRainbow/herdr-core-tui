"""Create worktrees for new and existing branches through the interactive dialog."""
import fcntl
import json
import os
import pathlib
import pty
import select
import shutil
import struct
import subprocess
import termios
import time

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herdr-tui"
core = Core("worktree-create")
repo = core.root / core.root.name
repo.mkdir()
checkout_base = pathlib.Path.home() / ".herdr/worktrees" / repo.name
assert not checkout_base.exists(), checkout_base
process = master = slave = None
try:
    def git(*args):
        return subprocess.check_output(["git", "-C", str(repo), *args]).decode().strip()

    git("init", "-qb", "main")
    git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "initial", "--allow-empty")
    for branch in ("existing-key", "existing-mouse"):
        git("branch", branch)
    original_commit = git("rev-parse", "HEAD")
    source = json.loads(subprocess.check_output([str(binary), "do", "space.new", json.dumps({"folder": str(repo), "argv": ["sh", "-c", "sleep 180"]})], env=core.env, timeout=10))
    source_space = source["terminal"]["labels"]["herdr.space"]
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
    process = subprocess.Popen([str(binary), "--instance", "worktree-create"], env=core.env, stdin=slave, stdout=slave, stderr=slave)

    def read_for(seconds=.05):
        output = b""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], .01)[0]:
                output += os.read(master, 65536)
        return output

    def wait_for(marker):
        output = b""
        end = time.monotonic() + 10
        while marker not in output and time.monotonic() < end:
            output += read_for()
        assert marker in output, output[-2000:]
        return output

    def view(command, args):
        result = subprocess.run([str(binary), "--instance", "worktree-create", "view", command, json.dumps(args)], env=core.env, capture_output=True, timeout=5)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return json.loads(result.stdout)

    def open_dialog(branch):
        view("space.focus", {"id": source_space})
        read_for()
        os.write(master, b"\x02G")
        output = wait_for(b"new worktree")
        assert b"new branch" in output and b"existing branch" in output, output
        os.write(master, b"\x15" + branch.encode())
        read_for()

    def verify(branch, count, path=None):
        path = path or checkout_base / branch
        end = time.monotonic() + 10
        while time.monotonic() < end:
            read_for()
            document = core.rpc("doc.get", {"name": "herdr-app"})
            spaces = document["value"]["spaces"]
            space = next((space for space in spaces if space.get("folder") == str(path)), None)
            state = view("state", {})
            pane = next((pane for pane in state["panes"] if pane["id"] == state["view"]["focused"]), None)
            if len(spaces) == count and space and state["document_available"] and state["document_version"] == document["version"] and state["view"]["active_space"] == space["id"] and pane:
                break
        else:
            raise AssertionError((spaces, state))
        assert subprocess.check_output(["git", "-C", str(path), "branch", "--show-current"]).decode().strip() == branch
        assert git("rev-parse", branch) == original_commit
        assert pathlib.Path(core.rpc("terminal.get", {"terminal": pane["terminal"]})["cwd"]) == path

    wait_for(b"main")
    open_dialog("new-ui")
    os.write(master, b"\x1b[B\x1b[D\x1b[C\x1b[A\x15new-default\r")
    verify("new-default", 2)

    open_dialog("existing-key")
    os.write(master, b"\r")
    wait_for(b"Git operation failed")
    assert not (checkout_base / "existing-key").exists()
    os.write(master, b"\t\r")
    verify("existing-key", 3)

    open_dialog("existing-mouse")
    os.write(master, b"\x1b[<0;55;9M\x1b[<0;55;9m")
    read_for()
    os.write(master, b"\r")
    verify("existing-mouse", 4)
    custom_path = repo / "custom checkout"
    open_dialog("custom-draft")
    os.write(master, b"\x1b[B\x15\r")
    wait_for(b"checkout path is required")
    assert not (checkout_base / "custom-draft").exists()
    os.write(master, b"\x1b[200~" + str(custom_path).encode() + b"\x1b[201~")
    read_for()
    os.write(master, b"\x1b[A\x15custom-final\r")
    verify("custom-final", 5, custom_path)

    mouse_path = repo / "mouse checkout"
    git("branch", "custom-existing")
    open_dialog("custom-existing")
    os.write(master, b"\t\x1b[<0;30;14M\x1b[<0;30;14m")
    read_for()
    os.write(master, b"\x15" + str(mouse_path).encode())
    read_for()
    os.write(master, b"\x1b[<0;30;11M\x1b[<0;30;11m")
    read_for()
    os.write(master, b"\x15custom-existing\r")
    verify("custom-existing", 6, mouse_path)

    for count, branch, relative, existing in (
        (7, "relative-new", "../custom-checkout", False),
        (8, "relative-existing", "../existing 'checkout' $(literal)", True),
    ):
        if existing:
            git("branch", branch)
        open_dialog(branch)
        os.write(master, (b"\t" if existing else b"") + b"\x1b[B\x15" + relative.encode() + b"\r")
        path = (repo / relative).resolve()
        verify(branch, count, path)
    assert len(git("worktree", "list", "--porcelain").split("worktree ")) - 1 == 8
    os.write(master, b"\x02q")
    assert process.wait(timeout=5) == 0
    print("PASS: default paths follow branch edits, new/existing branches, custom paths via keyboard/paste/mouse, empty path retry, custom paths survive branch edits, relative paths open and focus shared spaces, branch commits preserved")
finally:
    if process and process.poll() is None:
        process.kill()
        process.wait()
    if master is not None:
        os.close(master)
        os.close(slave)
    # The unique repository name confines default checkout paths to this test's directory.
    if checkout_base.exists():
        shutil.rmtree(checkout_base)
    core.close()
