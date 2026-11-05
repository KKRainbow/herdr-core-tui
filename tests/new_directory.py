"""New workspaces inherit the selected pane's live directory."""
import fcntl
import json
import os
import pathlib
import pty
import select
import shlex
import struct
import subprocess
import termios
import time

from fixture import Core

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/herdr-tui"
for prompt in (False, True):
    core = Core("new-directory", {"SHELL": "/bin/sh", "PS1": "READY> "})
    process = master = slave = None
    try:
        config = pathlib.Path(core.env["XDG_CONFIG_HOME"]) / "herdr-tui" / "config.toml"
        config.parent.mkdir(parents=True)
        config.write_text(f'[ui]\nprompt_new_workspace_name = {str(prompt).lower()}\n[terminal]\ndefault_shell = "/bin/sh"\n')
        first_folder = core.root / "first"
        second_folder = core.root / "second"
        nested = second_folder / "changed directory"
        explicit = core.root / "explicit"
        for folder in (first_folder, nested, explicit):
            folder.mkdir(parents=True)

        def command(name, args):
            return json.loads(subprocess.check_output([str(binary), "do", name, json.dumps(args)], env=core.env, timeout=15))

        def view(name, args=None):
            return json.loads(subprocess.check_output([str(binary), "--instance", "directory", "view", name, json.dumps(args or {})], env=core.env, timeout=10))

        def spaces():
            return core.rpc("doc.get", {"name": "herdr-app"})["value"]["spaces"]

        first = command("space.new", {"folder": str(first_folder)})
        second = command("pane.split", {"id": first["pane"], "folder": str(second_folder)})
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        process = subprocess.Popen([str(binary), "--instance", "directory"], env=core.env, cwd=core.root, stdin=slave, stdout=slave, stderr=slave)
        output = bytearray()

        def until(predicate, what):
            end = time.monotonic() + 10
            while time.monotonic() < end:
                while select.select([master], [], [], 0)[0]:
                    output.extend(os.read(master, 65536))
                if predicate():
                    return
                time.sleep(.02)
            raise AssertionError((what, spaces(), bytes(output[-1000:])))

        until(lambda: b"agents" in output, "workspace rendered")
        until(lambda: view("state")["metrics"]["attached"] == 2, "both panes attached")
        address = second["terminal"]["address"]
        until(lambda: core.rpc("terminal.read", {"terminal": address})["text"].rstrip().endswith("READY>"), "shell ready")
        core.rpc("terminal.send", {"terminal": address, "text": "cd " + shlex.quote(str(nested))})
        until(lambda: (core.rpc("terminal.get", {"terminal": address}).get("foreground") or {}).get("cwd") == str(nested), "shell changed directory")

        def new_from(pane, expected, count):
            view("pane.focus", {"id": pane})
            output.clear()
            os.write(master, b"\x02N")
            if prompt:
                until(lambda: b"new workspace" in output, "name prompt")
                os.write(master, b"Created\r")
            until(lambda: len(spaces()) == count, "workspace created")
            space = spaces()[-1]
            assert space["folder"] == str(expected), space
            terminal = space["tabs"][0]["root"]["children"][0]["content"]["terminal"]
            assert core.rpc("terminal.get", {"terminal": terminal})["cwd"] == str(expected)
            def focused():
                state = view("state")
                return state["document_available"] and state["view"]["active_space"] == space["id"] and len(state["panes"]) == 1
            until(focused, "new workspace focused and document available")

        new_from(second["pane"], nested, 2)
        new_from(first["pane"], first_folder, 3)
        created = command("space.new", {"folder": str(explicit)})
        assert created["terminal"]["cwd"] == str(explicit)
        view("quit")
        assert process.wait(timeout=5) == 0
    finally:
        if process and process.poll() is None:
            process.kill()
            process.wait()
        if master is not None:
            os.close(master)
            os.close(slave)
        core.close()
print("PASS: new workspace uses selected pane's live cwd, focus changes, name prompt on/off, explicit folder")
