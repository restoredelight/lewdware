import subprocess,time,pathlib
base=pathlib.Path('/home/restoredelight/projects/lewdware'); evidence=base/'deploy/vm/reports/evidence/ubuntu-wallpaper-2026-11-09'
def vm(*args):
 r=subprocess.run([str(base/'deploy/vm/vmctl'),'--vm','ubuntu-24.04',*args],capture_output=True,text=True); print(r.stdout,r.stderr,flush=True)
 with (evidence/'transcript.txt').open('a') as f: f.write(str(args)+'\n'+r.stdout+r.stderr)
 if r.returncode: raise RuntimeError(r.returncode)
 return r.stdout
def guest(s): return vm('ssh','--',s)
guest('python3 ~/wallpaper-guest.py start explicit')
time.sleep(3); vm('shot',str(evidence/'explicit-A.png')); guest('python3 ~/wallpaper-guest.py status')
time.sleep(14); vm('shot',str(evidence/'explicit-B.png')); guest('python3 ~/wallpaper-guest.py status')
time.sleep(15); vm('shot',str(evidence/'explicit-reset.png')); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check')
guest('python3 ~/wallpaper-guest.py stop'); time.sleep(1); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check')
guest('python3 ~/wallpaper-guest.py start hold'); time.sleep(3); guest('python3 ~/wallpaper-guest.py status')
vm('key','shift+esc'); time.sleep(2); guest('python3 ~/wallpaper-guest.py status')
guest('python3 ~/wallpaper-guest.py StopSession'); time.sleep(2); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check'); vm('shot',str(evidence/'graceful-stop-restored.png'))
