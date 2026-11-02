"""Measure TUI-only CPU, RSS, redraws, parsing, and RPC responsiveness."""
import fcntl,json,os,pathlib,pty,select,struct,subprocess,termios,threading,time
from fixture import Core
binary=pathlib.Path(os.environ.get("TUI_TEST_BINARY",pathlib.Path(__file__).resolve().parents[1]/"target/release/herder-tui")).resolve()
mode=os.environ.get("TUI_PERF_MODE","scroll");assert mode in ("scroll","repaint","metadata")
seconds=float(os.environ.get("TUI_PERF_SECONDS","5"));results=[]
for count in ([int(os.environ["TUI_PERF_PANES"])] if "TUI_PERF_PANES" in os.environ else (1,15)):
 core=Core("perf"+str(count));master=slave=process=None;stop=threading.Event()
 try:
  trigger=core.root/"output"
  program="import os,time\ni=0\nwhile True:\n if os.path.exists("+repr(str(trigger))+"):\n  print('fixture %08d '%i+'x'*60,flush=True);i+=1\n time.sleep(.02)"
  if mode=="repaint":
   program="import os,time\nprint(('fixture '+ 'x'*60+'\\r\\n')*40,end='',flush=True)\ni=0\nwhile True:\n if os.path.exists("+repr(str(trigger))+"):\n  print('\\x1b[2;1Hfixture %08d '%i+'x'*60,end='',flush=True);i+=1\n time.sleep(.02)"
  if mode=="metadata":program="import time; print('fixture',flush=True); time.sleep(180)"
  terminals=[core.rpc("terminal.run",{"argv":["python3","-u","-c",program],"no_parent":True}) for _ in range(count)]
  idle=core.rpc("terminal.run",{"argv":["sh","-c","sleep 180"],"no_parent":True})
  panes=[{"id":"p"+str(i),"type":"block","content":{"terminal":t["address"]}}for i,t in enumerate(terminals)]
  columns=[{"id":"col"+str(c),"type":"column","children":panes[c::3]}for c in range(min(3,count))]
  document={"spaces":[{"id":"s","name":"Profile","folder":str(core.root),"tabs":[{"id":"bench","name":"Bench","root":{"id":"bench","type":"row","children":columns}},{"id":"idle","name":"Idle","root":{"id":"idle","type":"row","children":[{"id":"idle-pane","type":"block","content":{"terminal":idle["address"]}}]}}]}]}
  current=core.rpc("doc.get",{"name":"herdr-app"});core.rpc("doc.set",{"name":"herdr-app","version":current["version"],"value":document})
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",50,180,0,0))
  process=subprocess.Popen([str(binary),"--session",core.env["HC_SESSION"],"--instance","bench"],env=core.env,stdin=slave,stdout=slave,stderr=slave)
  def drain():
   while not stop.is_set():
    if select.select([master],[],[],.05)[0]:
     try:os.read(master,65536)
     except OSError:return
  thread=threading.Thread(target=drain);thread.start()
  def view(command,args=None):return json.loads(subprocess.check_output([str(binary),"--session",core.env["HC_SESSION"],"--instance","bench","view",command,json.dumps(args or {})],env=core.env,timeout=10))
  time.sleep(1)
  def sample(phase):
   state=view("state");assert state["connected"] and state["document_available"],state;before=state["metrics"];stat=pathlib.Path(f"/proc/{process.pid}/stat").read_text().split();ticks=int(stat[13])+int(stat[14]);start=time.monotonic();latencies=[]
   while time.monotonic()-start<seconds:
    t=time.monotonic()
    if mode=="metadata" and phase=="visible":core.rpc("terminal.label",{"terminal":terminals[0]["address"],"labels":{"bench":str(len(latencies))}})
    else:core.rpc("terminal.list")
    latencies.append((time.monotonic()-t)*1000);time.sleep(.02 if mode=="metadata" else .1)
   elapsed=time.monotonic()-start;stat=pathlib.Path(f"/proc/{process.pid}/stat").read_text().split();cpu=((int(stat[13])+int(stat[14])-ticks)/os.sysconf("SC_CLK_TCK"))/elapsed*100
   rss=int(pathlib.Path(f"/proc/{process.pid}/statm").read_text().split()[1])*os.sysconf("SC_PAGE_SIZE")/1048576
   after=view("state")["metrics"];result={"mode":mode,"panes":count,"phase":phase,"seconds":round(elapsed,2),"cpu_percent":round(cpu,2),"rss_mib":round(rss,2),"renders":after["renders"]-before["renders"],"parsed_bytes":after["parsed_bytes"]-before["parsed_bytes"],"attached":after["attached"],"rpc_p95_ms":round(sorted(latencies)[int(len(latencies)*.95)],2)};result.update({key:after[key]-before[key] for key in ("snapshot_builds","layout_builds") if key in after});results.append(result);print(json.dumps(result),flush=True)
   if phase=="hidden":
    assert result["parsed_bytes"]==0 and result["renders"]==0,result
  if not os.environ.get("TUI_PERF_VISIBLE_ONLY"):sample("idle")
  trigger.touch();time.sleep(1);sample("visible")
  if not os.environ.get("TUI_PERF_VISIBLE_ONLY"):
   view("tab.focus",{"id":"idle"});time.sleep(.5);sample("hidden")
  view("quit");assert process.wait(timeout=5)==0
 finally:
  if process and process.poll()is None:process.kill();process.wait()
  stop.set()
  if 'thread'in locals():thread.join(timeout=1)
  if master is not None:os.close(master);os.close(slave)
  core.close()
if os.environ.get("TUI_PERF_OUTPUT"):pathlib.Path(os.environ["TUI_PERF_OUTPUT"]).write_text(json.dumps(results,indent=2)+"\n")
