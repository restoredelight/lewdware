import subprocess,time,pathlib
base=pathlib.Path('/home/restoredelight/projects/lewdware'); evidence=base/'deploy/vm/reports/evidence/ubuntu-wallpaper-2026-11-09'
def vm(*args):
 r=subprocess.run([str(base/'deploy/vm/vmctl'),'--vm','ubuntu-24.04',*args],capture_output=True,text=True); print(r.stdout,r.stderr,flush=True)
 with (evidence/'transcript.txt').open('a') as f: f.write(str(args)+'\n'+r.stdout+r.stderr)
 if r.returncode: raise RuntimeError(r.returncode)
 return r.stdout
def guest(s): return vm('ssh','--',s)
guest('python3 ~/wallpaper-guest.py start hold'); time.sleep(2)
guest('python3 ~/wallpaper-guest.py kill-engine'); time.sleep(0.3)
guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check'); vm('shot',str(evidence/'engine-crash-restored.png'))
time.sleep(3); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py stop'); time.sleep(1); guest('python3 ~/wallpaper-guest.py check')
guest('python3 ~/wallpaper-guest.py start hold'); time.sleep(2); guest('python3 ~/wallpaper-guest.py kill-both'); vm('shot',str(evidence/'stranded-wallpaper.png'))
vm('launch','/usr/lib/lewdware/lewdware-supervisor'); time.sleep(2); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check'); vm('shot',str(evidence/'startup-recovered.png'))
guest('python3 ~/wallpaper-guest.py permission off; python3 ~/wallpaper-guest.py start hold'); time.sleep(2); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check; python3 ~/wallpaper-guest.py stop'); time.sleep(1); guest('python3 ~/wallpaper-guest.py permission on')
