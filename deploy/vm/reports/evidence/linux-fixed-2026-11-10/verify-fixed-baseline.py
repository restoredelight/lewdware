"""Call after restoring/cleaning a new guest and shutting it down."""
import argparse,pathlib,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--vm',required=True);p.add_argument('--evidence',required=True);p.add_argument('--already-initialized',action='store_true');a=p.parse_args()
base=pathlib.Path('/home/restoredelight/projects/lewdware');ev=base/a.evidence;ev.mkdir(parents=True,exist_ok=True)
def run(tool,*args,check=True):
 r=subprocess.run([str(base/'deploy/vm'/tool),'--vm',a.vm,*args],capture_output=True,text=True)
 print(r.stdout+r.stderr,flush=True)
 with (ev/'baseline-transcript.txt').open('a') as f:f.write(repr((tool,*args))+'\n'+r.stdout+r.stderr)
 if check and r.returncode:raise RuntimeError((tool,args,r.returncode))
 return r.stdout,r.returncode
def vm(*args,**kwargs):return run('vmctl',*args,**kwargs)
def guest(cmd,**kwargs):return vm('ssh','--',cmd,**kwargs)
def down():
 _,code=guest('sudo systemctl poweroff',check=False);assert code in (0,255),code
 for _ in range(90):
  if 'stopped' in vm('status')[0]:return
  time.sleep(1)
 raise RuntimeError('Normal poweroff timed out')
if not a.already_initialized:run('vm-storage','init','--image','disk.qcow2','--image','OVMF_VARS.qcow2')
vm('up');vm('ready');time.sleep(15)
guest('set -e; printf reset-verification > ~/vmctl-reset-marker.txt; (cd ~/lewdware-fixed && sha256sum -c SHA256SUMS); test -f ~/lewdware-fixed-source/vm-source-manifest.json; test -f ~/wallpaper-fixture-fixed/wallpaper.lwpack')
vm('shot',str(ev/'baseline-first-boot.png'));down();run('vm-storage','reset','--discard')
vm('up');vm('ready');time.sleep(15)
guest('set -e; test ! -e ~/vmctl-reset-marker.txt; echo RESET_MARKER_ABSENT; (cd ~/lewdware-fixed && sha256sum -c SHA256SUMS); sha256sum ~/wallpaper-fixture-fixed/wallpaper.lwpack; test -s ~/lewdware-fixed-source/vm-source-manifest.json')
vm('shot',str(ev/'baseline-reset-boot.png'));down();run('vm-storage','reset','--discard');vm('status')
print('BASELINE RESET VERIFICATION PASSED',flush=True)
