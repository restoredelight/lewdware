import pathlib,subprocess,time
base=pathlib.Path('/home/restoredelight/projects/lewdware'); ev=base/'deploy/vm/reports/evidence/debian-xfce-2026-11-09'
def run(*args):
 r=subprocess.run([str(base/'deploy/vm'/args[0]),'--vm','debian-13-xfce',*args[1:]],capture_output=True,text=True)
 print(r.stdout+r.stderr,flush=True)
 with (ev/'baseline-transcript.txt').open('a') as f:f.write(repr(args)+'\n'+r.stdout+r.stderr)
 if r.returncode:raise RuntimeError(args)
 return r.stdout
def down():
 run('vmctl','down')
 for _ in range(60):
  if 'stopped' in run('vmctl','status'):return
  time.sleep(1)
 raise RuntimeError('shutdown timed out')
run('vmctl','up');run('vmctl','ready')
run('vmctl','ssh','--','printf baseline-reset-marker > ~/vmctl-reset-marker.txt; cat ~/vmctl-reset-marker.txt; dpkg-query -W lewdware; sha256sum ~/wallpaper-fixture/wallpaper.lwpack')
down();run('vm-storage','reset','--discard');run('vmctl','up');run('vmctl','ready')
run('vmctl','ssh','--','test ! -e ~/vmctl-reset-marker.txt && echo RESET_MARKER_ABSENT && dpkg-query -W lewdware && sha256sum ~/wallpaper-fixture/wallpaper.lwpack && test -s ~/wallpaper-test.lwmode && echo MODE_RETAINED')
run('vmctl','launch','/usr/bin/lewdware');time.sleep(8);run('vmctl','shot',str(ev/'reset-app.png'))
down();run('vm-storage','reset','--discard')
