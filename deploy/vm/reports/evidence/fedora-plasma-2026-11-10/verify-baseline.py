import pathlib,subprocess,time
base=pathlib.Path('/home/restoredelight/projects/lewdware'); ev=base/'deploy/vm/reports/evidence/fedora-plasma-2026-11-10'
def run(*args, allowed=(0,)):
 r=subprocess.run([str(base/'deploy/vm'/args[0]),'--vm','fedora-44-plasma',*args[1:]],capture_output=True,text=True)
 print(r.stdout+r.stderr,flush=True)
 with (ev/'baseline-transcript.txt').open('a') as f:f.write(repr(args)+'\n'+r.stdout+r.stderr)
 if r.returncode not in allowed:raise RuntimeError(args)
 return r.stdout
def down():
 run('vmctl','ssh','--','sudo systemctl poweroff',allowed=(0,255))
 for _ in range(60):
  if 'stopped' in run('vmctl','status'):return
  time.sleep(1)
 raise RuntimeError('shutdown timed out')
run('vmctl','up');run('vmctl','ready')
run('vmctl','ssh','--','printf baseline-reset-marker > ~/vmctl-reset-marker.txt; cat ~/vmctl-reset-marker.txt; rpm -q lewdware; sha256sum ~/wallpaper-fixture/wallpaper.lwpack')
down();run('vm-storage','reset','--discard');run('vmctl','up');run('vmctl','ready')
run('vmctl','ssh','--','test ! -e ~/vmctl-reset-marker.txt && echo RESET_MARKER_ABSENT && rpm -q lewdware && sha256sum ~/wallpaper-fixture/wallpaper.lwpack && test -s ~/wallpaper-test.lwmode && echo MODE_RETAINED')
run('vmctl','ssh','--', 'for i in $(seq 1 40); do systemctl --user is-active --quiet graphical-session.target && exit 0; sleep 1; done; exit 1')
run('vmctl','launch','/usr/bin/lewdware');time.sleep(12);run('vmctl','shot',str(ev/'reset-app.png'))
down();run('vm-storage','reset','--discard')
