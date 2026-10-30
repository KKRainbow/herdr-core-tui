"""Core-managed remote machine loss, offline edits, and resumed views."""
import fcntl,json,os,pathlib,pty,select,shlex,signal,struct,subprocess,termios,threading,time
from fixture import Core
binary=pathlib.Path(os.environ.get("TUI_TEST_BINARY",pathlib.Path(__file__).resolve().parents[1]/"target/release/herder-tui")).resolve()
local=Core("local");remote=Core("remote");process=None;master=slave=None;stop=threading.Event()
try:
 via=local.root/"remote-command";blocked=local.root/"blocked";pidfile=local.root/"transport.pid"
 via.write_text('#!/bin/sh\n[ ! -f '+shlex.quote(str(blocked))+' ] || exit 1\nprintf "%s" "$$" > '+shlex.quote(str(pidfile))+'\n'+'\n'.join('export '+key+'='+shlex.quote(remote.env[key])for key in ('HC_SESSION','XDG_RUNTIME_DIR','XDG_STATE_HOME','XDG_CONFIG_HOME'))+'\nexec sh -c "$1"\n');via.chmod(0o700)
 subprocess.run(["hc","machine","add","--via",str(via),"--name","fixture","--session",remote.env["HC_SESSION"]],env=local.env,check=True,capture_output=True,timeout=20)
 def cli(*args,check=True):
  result=subprocess.run([str(binary),"--session",local.env["HC_SESSION"],*args],env=local.env,capture_output=True,timeout=15)
  if check:assert result.returncode==0,result.stderr
  return json.loads(result.stdout)if result.returncode==0 else result
 created=cli("--machine","fixture","do","space.new",json.dumps({"name":"Remote","folder":str(remote.root),"argv":["sh","-c","printf REMOTE_VIEW; sleep 180"]}))
 address=created["terminal"]["address"];pid=local.rpc("terminal.get",{"terminal":address})["pid"]
 master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",24,100,0,0))
 process=subprocess.Popen([str(binary),"--session",local.env["HC_SESSION"],"--machine","fixture","--instance","remote"],env=local.env,stdin=slave,stdout=slave,stderr=slave)
 output=bytearray()
 def drain():
  while not stop.is_set():
   if select.select([master],[],[],.1)[0]:
    try:output.extend(os.read(master,65536))
    except OSError:return
 thread=threading.Thread(target=drain);thread.start()
 def until(predicate,seconds=20):
  end=time.monotonic()+seconds
  while time.monotonic()<end:
   if predicate():return
   time.sleep(.1)
  raise AssertionError("condition did not become true")
 until(lambda:b"REMOTE_VIEW"in output)
 blocked.touch();os.kill(int(pidfile.read_text()),signal.SIGTERM)
 until(lambda:local.rpc("machine.list")["machines"][0]["state"]!="connected")
 until(lambda:not cli("--instance","remote","view","state")["document_available"])
 rejected=cli("--machine","fixture","do","space.rename",json.dumps({"id":created["space"],"name":"MUST NOT WRITE"}),check=False)
 assert isinstance(rejected,subprocess.CompletedProcess)and rejected.returncode!=0
 document=remote.rpc("doc.get",{"name":"herdr-app"});document["value"]["spaces"][0]["name"]="Changed offline"
 remote.rpc("doc.set",{"name":"herdr-app","version":document["version"],"value":document["value"]})
 blocked.unlink()
 until(lambda:local.rpc("machine.list")["machines"][0]["state"]=="connected",30)
 until(lambda:cli("--instance","remote","view","state")["document_version"]>document["version"])
 assert cli("--instance","remote","view","state")["view"]["focused"]==created["pane"]
 assert local.rpc("terminal.get",{"terminal":address})["pid"]==pid,"remote loss relaunched process"
 assert cli("--instance","remote","view","quit")["ok"];assert process.wait(timeout=5)==0
 print("PASS: remote terminal, offline mutation rejection, document refresh, same process after reconnect")
finally:
 if process and process.poll()is None:process.kill();process.wait()
 stop.set()
 if 'thread'in locals():thread.join(timeout=1)
 if master is not None:os.close(master);os.close(slave)
 local.close();remote.close()
