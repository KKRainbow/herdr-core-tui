"""Viewer ownership, live divider drags, and in-band terminal size reports."""
import fcntl,json,os,pathlib,pty,re,select,struct,subprocess,sys,termios,threading,time
from fixture import Core
binary=pathlib.Path(os.environ.get("TUI_TEST_BINARY",pathlib.Path(__file__).resolve().parents[1]/"target/release/herdr-tui")).resolve()
core=Core("geometry");viewers=[];stop=threading.Event();reports=core.root/"size-reports"
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
 program="""
import os,sys,tty
tty.setraw(0)
with open(sys.argv[1], "ab", buffering=0) as reports:
 os.write(1, b"GEOMETRY\\x1b[?2048h")
 while True:
  data=os.read(0,4096)
  if not data:break
  reports.write(data)
"""
 created=cli("do","space.new",json.dumps({"name":"Geometry","folder":str(core.root),"argv":[sys.executable,"-c",program,str(reports)]}))
 address=created["terminal"]["address"]
 first=launch(100,24,["--instance","workspace"])
 def size():
  record=core.rpc("terminal.get",{"terminal":address});return record["cols"],record["rows"]
 def reported():
  data=reports.read_bytes() if reports.exists() else b""
  matches=re.findall(rb"\x1b\[48;(\d+);(\d+);\d+;\d+t",data)
  return (int(matches[-1][1]),int(matches[-1][0])) if matches else None
 until(lambda:size()==(73,23))
 until(lambda:reported()==(73,23))
 second=launch(60,16,[address]);until(lambda:size()==(60,16))
 until(lambda:reported()==(60,16))
 for _ in range(25):assert size()==(60,16),size();time.sleep(.1)
 os.write(first,b"a");until(lambda:size()==(73,23))
 until(lambda:reported()==(73,23))
 for _ in range(25):assert size()==(73,23),size();time.sleep(.1)
 os.write(second,b"b");until(lambda:size()==(60,16))
 until(lambda:reported()==(60,16))
 right=cli("do","pane.split",json.dumps({"id":created["pane"],"axis":"row","argv":["sleep","60"]}))
 until(lambda:size()==(34,21) and reported()==(34,21))

 def mouse(button,x,y,release=False):
  os.write(first,f"\x1b[<{button};{x};{y}{'m' if release else 'M'}".encode())

 def document():
  return core.rpc("doc.get",{"name":"herdr-app"})

 def drag(start,steps):
  before=document()
  mouse(0,*start)

  for x,y,expected in steps:
   mouse(32,x,y)
   # The program receives every held position before the shared layout changes.
   until(lambda:size()==expected and reported()==expected)
   assert document()==before,(before,document())

  mouse(0,x,y,release=True)
  until(lambda:document()["version"]==before["version"]+1)
  until(lambda:size()==expected and reported()==expected)

  for _ in range(5):
   assert size()==expected and reported()==expected,(size(),reported())
   time.sleep(.1)

 drag((64,10),[(72,10,(42,21)),(59,10,(29,21))])
 root=document()["value"]["spaces"][0]["tabs"][0]["root"]
 assert abs(root["children"][0]["size"]["weight"]/sum(c["size"]["weight"] for c in root["children"])-32/74)<1e-9,root
 cli("do","pane.resize",json.dumps({"id":created["pane"],"next":right["pane"],"share":.5}))
 until(lambda:size()==(34,21) and reported()==(34,21))
 cli("do","pane.split",json.dumps({"id":created["pane"],"axis":"column","argv":["sleep","60"]}))
 until(lambda:size()==(34,10) and reported()==(34,10))
 drag((40,13),[(40,16,(34,13)),(40,11,(34,8))])
 os.write(first,b"\x02z");until(lambda:size()==(71,21) and reported()==(71,21))
 for _ in range(10):assert size()==(71,21),size();time.sleep(.1)
 os.write(first,b"\x02z");until(lambda:size()==(34,8) and reported()==(34,8))
 os.write(first,b"\x02q");os.write(second,b"\x11")
 for process,_,_,_ in viewers:assert process.wait(timeout=5)==0
 assert core.rpc("terminal.get",{"terminal":address})["state"]=="live"
 print("PASS: in-band reports on attach, split, live divider drags on both axes and nested panes, one layout write on release, zoom and unzoom, two viewer sizes, input transfers ownership, no background resize contention, detach keeps process")
finally:
 for process,_,_,_ in viewers:
  if process.poll()is None:process.kill();process.wait()
 stop.set()
 for _,master,slave,thread in viewers:thread.join(timeout=1);os.close(master);os.close(slave)
 core.close()
