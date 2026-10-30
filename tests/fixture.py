"""Isolated public-protocol core fixture for acceptance and profiling."""
import json,os,pathlib,select,subprocess,tempfile
class Core:
 def __init__(self,name):
  self.temporary=tempfile.TemporaryDirectory(prefix="tui-"+name+"-");self.root=pathlib.Path(self.temporary.name)
  self.env=dict(os.environ,HERDER_TUI_DISABLE_SOUND="1",HC_SESSION=name,XDG_RUNTIME_DIR=str(self.root/"run"),XDG_STATE_HOME=str(self.root/"state"),XDG_CONFIG_HOME=str(self.root/"config"),HERDR_APP_HOME=str(self.root/"app"),TERM="xterm-256color")
  pathlib.Path(self.env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
  self.proxy=subprocess.Popen(["hc","connect"],env=self.env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  self.seq=0
 def rpc(self,method,params=None):
  self.seq+=1;request={"jsonrpc":"2.0","id":self.seq,"method":method,"params":params or {}}
  self.proxy.stdin.write(json.dumps(request).encode()+b"\n");self.proxy.stdin.flush()
  if not select.select([self.proxy.stdout],[],[],15)[0]:raise TimeoutError(method)
  reply=json.loads(self.proxy.stdout.readline())
  if "error"in reply:raise RuntimeError(reply["error"])
  return reply["result"]
 def close(self):
  self.proxy.terminate();self.proxy.wait(timeout=5)
  subprocess.run(["hc","server","stop"],env=self.env,stdout=subprocess.DEVNULL,check=True,timeout=15)
  self.temporary.cleanup()
