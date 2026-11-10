import argparse,json,pathlib,shlex,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--vm',required=True);p.add_argument('--evidence',required=True);a=p.parse_args()
base=pathlib.Path('/home/restoredelight/projects/lewdware');evidence=base/a.evidence;evidence.mkdir(parents=True,exist_ok=True)
class IdleDaemonGone(RuntimeError):pass
def vm(*args,check=True):
 r=subprocess.run([str(base/'deploy/vm/vmctl'),'--vm',a.vm,*args],capture_output=True,text=True)
 with (evidence/'transcript.txt').open('a') as f:f.write(repr(args)+'\n'+r.stdout+r.stderr)
 print(r.stdout+r.stderr,flush=True)
 if check and r.returncode:
  if 'ConnectionRefusedError' in r.stderr:raise IdleDaemonGone((args,r.returncode))
  raise RuntimeError((args,r.returncode))
 return r.stdout
def guest(s):return vm('ssh','--',s)
def probe(cmd):
 out=guest('LW_WALLPAPER_EVIDENCE="$HOME/wallpaper-fixed-evidence" LW_WALLPAPER_FIXTURE="$HOME/wallpaper-fixture-fixed" LW_WALLPAPER_MODE="$HOME/lewdware-fixed/wallpaper-test.lwmode" python3 ~/wallpaper-probe-fixed.py '+cmd)
 return [json.loads(x) for x in out.splitlines() if x.startswith('{')]
def status():return probe('status')[0]['value']
def running():
 state=status();assert state['session']['type']=='Running',state
 guest('test "$(readlink /proc/'+str(state['session']['pid'])+'/exe)" = "$HOME/lewdware-fixed/bin/lewdware-engine"')
def idle():
 try:state=status()
 except IdleDaemonGone:
  guest("test ! -f ~/.local/share/lewdware/wallpaper-snapshot.json && ! pgrep -x lewdware-engine")
  guest("python3 - <<'END'\nimport json\nfrom pathlib import Path\nrows=[json.loads(line) for p in sorted((Path.home()/'.local/share/lewdware/logs').glob('lewdware-supervisor.jsonl.*')) for line in p.read_text().splitlines()]\nassert rows[-1]['message']=='no active session for 60s, exiting',rows[-1]\nprint('LOGGED_IDLE_EXIT_CONFIRMED',rows[-1]['timestamp'])\nEND")
  supervisor();state=status()
 assert state['session']['type']=='Idle',state
 guest('test ! -f ~/.local/share/lewdware/wallpaper-snapshot.json')
def shot(name):vm('shot',str(evidence/(name+'.png')))
def supervisor():
 vm('launch',guest('printf %s "$HOME"').strip()+'/lewdware-fixed/bin/lewdware-supervisor');time.sleep(1)
 guest("python3 - <<'END'\nimport runpy\nfrom pathlib import Path\nr=runpy.run_path(str(Path.home()/'wallpaper-probe-fixed.py'));pid=r['exact_pid']('lewdware-supervisor')\nassert Path(f'/proc/{pid}/exe').resolve()==Path.home()/'lewdware-fixed/bin/lewdware-supervisor'\nEND")
def restored():probe('check');idle()
supervisor();restored()
probe('start hold');time.sleep(3);probe('kill-both');shot('stranded');supervisor();time.sleep(2);restored();shot('startup-recovered')
probe('permission off');probe('start hold');time.sleep(3);probe('check');probe('stop');time.sleep(1);restored();probe('permission on')
probe('start missing');time.sleep(3);probe('check');probe('stop');time.sleep(1);restored()
probe('restore-original');time.sleep(2);shot('original-restored')
vm('pull','~/wallpaper-fixed-evidence',str(evidence/'guest'))
vm('pull','~/.local/share/lewdware/logs',str(evidence/'logs'))
print('FIXED CORE RESTORATION SUITE PASSED',flush=True)
