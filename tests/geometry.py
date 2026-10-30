"""Two viewers follow core geometry without idle resize contention."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,termios,threading,time
from fixture import Core
binary=pathlib.Path(os.environ.get("TUI_TEST_BINARY",pathlib.Path(__file__).resolve().parents[1]/"target/release/herder-tui")).resolve()
core=Core("geometry");viewers=[];stop=threading.Event()
def cli(*args):return json.loads(subprocess.check_output([str(binary),"--session",core.env["HC_SESSION"],*args],env=core.env,timeout=10))
def until(predicate):
 end=time.monotonic()+10
 while time.monotonic()<end:
  if predicate():return
  time.sleep(.1)
 raise AssertionError("geometry did not converge")
def launch(cols,rows,args):
 master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",rows,cols,0,0))
 process=subprocess.Popen([str(binary),"--session",core.env["HC_SESSION"],*args],env=core.env,stdin=slave,stdout=slave,stderr=slave)
 def drain():
  while not stop.is_set():
   if select.select([master],[],[],.05)[0]:
    try:os.read(master,65536)
    except OSError:return
 thread=threading.Thread(target=drain);thread.start();viewers.append((process,master,slave,thread));return master
try:
 created=cli("do","space.new",json.dumps({"name":"Geometry","folder":str(core.root),"argv":["sh","-c","printf GEOMETRY; cat"]}))
 address=created["terminal"]["address"]
 first=launch(100,24,["--instance","workspace"])
 def size():
  record=core.rpc("terminal.get",{"terminal":address});return record["cols"],record["rows"]
 until(lambda:size()==(73,23))
 second=launch(60,16,[address]);until(lambda:size()==(60,16))
 for _ in range(25):assert size()==(60,16),size();time.sleep(.1)
 os.write(first,b"a");until(lambda:size()==(73,23))
 for _ in range(25):assert size()==(73,23),size();time.sleep(.1)
 os.write(second,b"b");until(lambda:size()==(60,16))
 os.write(first,b"\x02q");os.write(second,b"\x11")
 for process,_,_,_ in viewers:assert process.wait(timeout=5)==0
 assert core.rpc("terminal.get",{"terminal":address})["state"]=="live"
 print("PASS: two different view sizes, input transfers ownership, no background resize contention, detach keeps process")
finally:
 for process,_,_,_ in viewers:
  if process.poll()is None:process.kill();process.wait()
 stop.set()
 for _,master,slave,thread in viewers:thread.join(timeout=1);os.close(master);os.close(slave)
 core.close()
