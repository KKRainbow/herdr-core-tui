"""Standalone commands and unambiguous live-view targeting."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,tempfile,termios,time
binary=pathlib.Path(os.environ.get("TUI_TEST_BINARY",pathlib.Path(__file__).resolve().parents[1]/"target/debug/herder-tui")).resolve()
with tempfile.TemporaryDirectory(prefix="tui-cli-")as root:
 env=dict(os.environ,HC_SESSION="tui-cli",XDG_RUNTIME_DIR=root+"/run",XDG_STATE_HOME=root+"/state",XDG_CONFIG_HOME=root+"/config",HERDR_APP_HOME=root+"/app",TERM="xterm-256color")
 pathlib.Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
 def cli(*args):return json.loads(subprocess.check_output([str(binary),"--session",env["HC_SESSION"],*args],env=env,cwd=root,timeout=10))
 viewers=[]
 try:
  first=cli("do","space.new",json.dumps({"name":"CLI","folder":root,"argv":["sh","-c","sleep 180"]}))
  labels=first["terminal"]["labels"];space=labels["herdr.space"];tab1=labels["herdr.tab"]
  second=cli("do","tab.new",json.dumps({"space":space,"name":"Second","argv":["sh","-c","sleep 180"]}))
  tab2=second["terminal"]["labels"]["herdr.tab"]
  for name in ["one","two"]:
   master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",24,100,0,0));original=termios.tcgetattr(slave)
   process=subprocess.Popen([str(binary),"--session",env["HC_SESSION"],"--instance",name],env=env,cwd=root,stdin=slave,stdout=slave,stderr=slave)
   viewers.append((name,process,master,slave,original))
   end=time.monotonic()+10;data=b""
   while b"Instance" not in data and time.monotonic()<end:
    if select.select([master],[],[],.1)[0]:data+=os.read(master,65536)
   assert b"Instance" in data,data[-1000:]
  assert len(cli("instances")["instances"])==2
  assert cli("--instance","one","view","tab.focus",json.dumps({"id":tab2}))["ok"]
  assert cli("--instance","one","view","zoom")["ok"]
  one=cli("--instance","one","view","state");two=cli("--instance","two","view","state")
  assert one["view"]["active_tab"]==tab2 and one["view"]["zoomed"] is not None
  assert two["view"]["active_tab"]==tab1 and two["view"]["zoomed"] is None
  rejected=subprocess.run([str(binary),"--session",env["HC_SESSION"],"view","zoom"],env=env,capture_output=True)
  assert rejected.returncode!=0 and b"--instance" in rejected.stderr
  duplicate=subprocess.run([str(binary),"--session",env["HC_SESSION"],"--instance","one"],env=env,capture_output=True)
  assert duplicate.returncode!=0
  for name,process,master,slave,original in viewers:
   if name=="two":
    # Force a handled state-save error after terminal setup.
    view_path=pathlib.Path(root)/"state/herder-tui"/("view-"+(env["HC_SESSION"]+":two").encode().hex()+".json")
    view_path.mkdir(parents=True)
   assert cli("--instance",name,"view","quit")["ok"]
   assert process.wait(timeout=5)==(1 if name=="two" else 0)
   assert termios.tcgetattr(slave)==original
  assert cli("instances")["instances"]==[]
  record=json.loads(subprocess.check_output(["hc","get",first["terminal"]["address"]],env=env))
  assert record["state"]=="live"
  print("PASS: headless structure, standalone binary, two independent views, explicit targeting, endpoint cleanup, handled-error restoration, surviving terminals")
 finally:
  for _,process,master,slave,_ in viewers:
   if process.poll()is None:process.kill();process.wait()
   os.close(master);os.close(slave)
  subprocess.run(["hc","server","stop"],env=env,check=True,stdout=subprocess.DEVNULL)
