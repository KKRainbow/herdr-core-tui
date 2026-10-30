"""Workspace edits and independent view state against an isolated unchanged app."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,termios,time
root=os.environ["TUI_APP_TEST_ROOT"]
assert root.startswith("/tmp/")
env=dict(os.environ,HERDR_APP_HOME=root+"/app",XDG_RUNTIME_DIR=root+"/runtime",XDG_STATE_HOME=root+"/state",XDG_CONFIG_HOME=root+"/config",TERM="xterm-256color")
def app():return json.loads(subprocess.check_output(["herdr-app","ls"],env=env))
def active():return [(s["id"],t["id"])for s in app()["spaces"]for t in s["tabs"]if t["active"]]
before=active()
name="TUI test "+str(time.time_ns())
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",32,120,0,0));original=termios.tcgetattr(slave)
binary=pathlib.Path(__file__).resolve().parents[1]/"target/debug/herder-tui"
p=subprocess.Popen([str(binary),"--session",env["HC_SESSION"]],env=env,stdin=slave,stdout=slave,stderr=slave)
def drain(seconds=.3):
 data=b"";end=time.monotonic()+seconds
 while time.monotonic()<end:
  if select.select([master],[],[],.05)[0]:data+=os.read(master,65536)
 return data
def send(data):os.write(master,data);return drain()
def until(predicate):
 deadline=time.monotonic()+10
 while time.monotonic()<deadline:
  drain(.1)
  if predicate():return
 raise AssertionError("app did not reflect TUI change")
try:
 assert b"Ctrl+B" in drain(1)
 send(b"\x02t")
 send(name.encode()+b"\tprintf WORKSPACE_TEST; cat\r")
 until(lambda:any(t["name"]==name for s in app()["spaces"]for t in s["tabs"]))
 assert active()==before,"TUI changed app tab focus"
 tab=next(t for s in app()["spaces"]for t in s["tabs"]if t["name"]==name)
 # Open the pane context menu and select Rename tab with the mouse.
 send(b'\x1b[<2;40;10M')
 send(b'\x1b[<0;20;15M')
 send((name+" renamed").encode()+b'\r')
 until(lambda:any(t["name"]==name+" renamed" for s in app()["spaces"]for t in s["tabs"]))
 send(b'\x02%')
 send(b'\tprintf SPLIT_TEST; cat\r')
 until(lambda:any(len(t["terminals"])==2 for s in app()["spaces"]for t in s["tabs"]if t["id"]==tab["id"]))
 send(b'\x02z')
 assert active()==before,"TUI zoom changed app focus"
 send(b'\x02x')
 send(b'y')
 until(lambda:any(len(t["terminals"])==1 for s in app()["spaces"]for t in s["tabs"]if t["id"]==tab["id"]))
 send(b'\x11');assert p.wait(timeout=5)==0
 assert termios.tcgetattr(slave)==original
 assert active()==before
 print("PASS: TUI tab creation, split, close, independent app focus/zoom, host restore")
finally:
 if p.poll()is None:p.kill();p.wait()
 os.close(master);os.close(slave)
