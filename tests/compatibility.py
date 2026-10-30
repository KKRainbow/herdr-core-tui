"""Preserve future app content and stop retrying completed terminals."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,termios,threading,time
from fixture import Core
binary=pathlib.Path(os.environ.get("TUI_TEST_BINARY",pathlib.Path(__file__).resolve().parents[1]/"target/release/herder-tui")).resolve()
core=Core("compatibility");process=None;master=slave=None;stop=threading.Event()
def cli(*args):return json.loads(subprocess.check_output([str(binary),"--session",core.env["HC_SESSION"],*args],env=core.env,timeout=10))
try:
 created=cli("do","space.new",json.dumps({"name":"Compatibility","folder":str(core.root),"argv":["sh","-c","printf FINISHED_FIXTURE; sleep 180"]}))
 current=core.rpc("doc.get",{"name":"herdr-app"});document=current["value"]
 unknown={"id":"future-pane","type":"block","content":{"mod":"future-widget","settings":{"opaque":[1,"two"]}},"future-property":{"keep":True}}
 document["spaces"][0]["tabs"][0]["root"]["children"].append(unknown)
 document["future-root"]={"opaque":True}
 core.rpc("doc.set",{"name":"herdr-app","version":current["version"],"value":document})
 cli("do","space.rename",json.dumps({"id":created["space"],"name":"Renamed"}))
 after=core.rpc("doc.get",{"name":"herdr-app"})["value"]
 assert after["spaces"][0]["tabs"][0]["root"]["children"][-1]==unknown
 assert after["future-root"]==document["future-root"]
 master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",30,120,0,0))
 process=subprocess.Popen([str(binary),"--session",core.env["HC_SESSION"],"--instance","compat"],env=core.env,stdin=slave,stdout=slave,stderr=slave)
 output=bytearray()
 def drain():
  while not stop.is_set():
   if select.select([master],[],[],.05)[0]:
    try:output.extend(os.read(master,65536))
    except OSError:return
 thread=threading.Thread(target=drain);thread.start()
 end=time.monotonic()+10
 while b"Unsupported"not in output and time.monotonic()<end:time.sleep(.1)
 assert b"Unsupported"in output,output[-2000:]
 core.rpc("terminal.kill",{"terminal":created["terminal"]["address"]})
 time.sleep(1)
 before=cli("--instance","compat","view","state")["metrics"]
 time.sleep(3)
 after=cli("--instance","compat","view","state")["metrics"]
 assert before["renders"]==after["renders"] and after["attached"]==0,(before,after)
 cli("--instance","compat","view","quit");assert process.wait(timeout=5)==0
 output.clear()
 process=subprocess.Popen([str(binary),"--session",core.env["HC_SESSION"],"--instance","compat"],env=core.env,stdin=slave,stdout=slave,stderr=slave)
 end=time.monotonic()+10
 while b"FINISHED_FIXTURE"not in output and time.monotonic()<end:time.sleep(.1)
 assert b"FINISHED_FIXTURE"in output,output[-2000:]
 cli("--instance","compat","view","quit");assert process.wait(timeout=5)==0
 print("PASS: unknown mod placeholder, lossless unrelated edit, finished terminal retained without retry/redraw loop")
finally:
 if process and process.poll()is None:process.kill();process.wait()
 stop.set()
 if 'thread'in locals():thread.join(timeout=1)
 if master is not None:os.close(master);os.close(slave)
 core.close()
