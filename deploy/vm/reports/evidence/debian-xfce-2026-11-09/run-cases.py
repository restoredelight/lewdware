import argparse, pathlib, subprocess,time
p=argparse.ArgumentParser();p.add_argument('--vm',required=True);p.add_argument('--evidence',required=True);a=p.parse_args()
base=pathlib.Path('/home/restoredelight/projects/lewdware'); evidence=base/a.evidence;evidence.mkdir(parents=True,exist_ok=True)
def vm(*args):
 r=subprocess.run([str(base/'deploy/vm/vmctl'),'--vm',a.vm,*args],capture_output=True,text=True)
 print(r.stdout+r.stderr,flush=True)
 with (evidence/'transcript.txt').open('a') as f:f.write(repr(args)+'\n'+r.stdout+r.stderr)
 if r.returncode:raise RuntimeError((args,r.returncode))
 return r.stdout
def guest(s):return vm('ssh','--',s)
def probe(cmd):return guest('python3 ~/wallpaper-probe.py '+cmd)
def shot(name):return vm('shot',str(evidence/(name+'.png')))
def supervisor():
 vm('launch','/usr/lib/lewdware/lewdware-supervisor');time.sleep(.5)
supervisor();probe('start explicit');time.sleep(3);probe('status');shot('explicit-A')
time.sleep(13);probe('status');shot('explicit-B')
time.sleep(14);probe('check');probe('status');shot('explicit-reset');probe('stop');time.sleep(1);probe('check');probe('status')
probe('start hold');time.sleep(2);vm('key','shift+esc');time.sleep(1);probe('status')
# Always explicitly stop too: a known Wayland shortcut failure must not block the rest.
probe('stop');time.sleep(1);probe('check');shot('global-or-immediate-stop')
probe('start hold');time.sleep(2);probe('StopSession');time.sleep(1);probe('check');probe('status')
probe('start hold');time.sleep(2)
guest('python3 ~/wallpaper-probe.py kill-engine && sleep 0.3 && python3 ~/wallpaper-probe.py check && python3 ~/wallpaper-probe.py status && python3 ~/wallpaper-probe.py stop')
time.sleep(1);probe('check')
probe('start hold');time.sleep(2);probe('kill-both');shot('stranded');supervisor();time.sleep(1);probe('check');probe('status');shot('startup-recovered')
probe('permission off');probe('start hold');time.sleep(2);probe('check');probe('status');probe('stop');time.sleep(1);probe('permission on')
probe('start missing');time.sleep(2);probe('check');probe('status');probe('stop');time.sleep(1);probe('check');probe('status')
print('RESTORATION CASES FINISHED',flush=True)
