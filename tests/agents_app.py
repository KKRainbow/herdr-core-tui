"""Reported agents remain navigable without an app pane."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,time,termios
root=os.environ["TUI_APP_TEST_ROOT"];assert root.startswith("/tmp/")
env=dict(os.environ,HERDER_TUI_DISABLE_SOUND="1",HERDR_APP_HOME=root+"/app",XDG_RUNTIME_DIR=root+"/runtime",XDG_STATE_HOME=root+"/state",XDG_CONFIG_HOME=root+"/config",TERM="xterm-256color")
config=pathlib.Path(root+"/config/herder-tui/config.json");config.parent.mkdir(parents=True,exist_ok=True)
config.write_text(json.dumps({"keys":{"next_agent":"prefix+a"}}))
record=json.loads(subprocess.check_output(["hc","run","--bg","--json","sh","-c","printf '\\033]7501;state=blocked\\033\\\\'; printf DETACHED_AGENT; sleep 60"],env=env))
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",32,120,0,0))
binary=pathlib.Path(__file__).resolve().parents[1]/"target/debug/herder-tui"
p=subprocess.Popen([str(binary),"--session",env["HC_SESSION"]],env=env,stdin=slave,stdout=slave,stderr=slave)
def wait(marker):
 output=b"";end=time.monotonic()+10
 while marker not in output and time.monotonic()<end:
  if select.select([master],[],[],.1)[0]:output+=os.read(master,65536)
 assert marker in output,output[-2000:]
try:
 wait(b"blocked")
 output=b"";end=time.monotonic()+10
 while b"DETACHED_AGENT"not in output and time.monotonic()<end:
  os.write(master,b'\x02a')
  tick=time.monotonic()+.3
  while time.monotonic()<tick:
   if select.select([master],[],[],.05)[0]:output+=os.read(master,65536)
 assert b"DETACHED_AGENT"in output,output[-2000:]
 os.write(master,b'\x02q');assert p.wait(timeout=5)==0
 print("PASS: reported status and keyboard navigation for an agent without a pane")
finally:
 if p.poll()is None:p.kill();p.wait()
 os.close(master);os.close(slave)
 subprocess.run(["hc","kill",record["address"]],env=env,check=True,stdout=subprocess.DEVNULL)
