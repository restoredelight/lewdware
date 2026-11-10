import argparse,json,pathlib,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--vm',required=True);p.add_argument('--desktop',required=True,choices=['gnome','xfce','kde','cinnamon','mate','sway']);p.add_argument('--evidence',required=True);a=p.parse_args()
base=pathlib.Path('/home/restoredelight/projects/lewdware');ev=base/a.evidence;ev.mkdir(parents=True,exist_ok=True)
def vm(*args):
 r=subprocess.run([str(base/'deploy/vm/vmctl'),'--vm',a.vm,*args],capture_output=True,text=True)
 print(r.stdout+r.stderr,flush=True)
 with (ev/'shortcut-transcript.txt').open('a') as f:f.write(repr(args)+'\n'+r.stdout+r.stderr)
 if r.returncode:raise RuntimeError(args)
 return r.stdout
def guest(s):return vm('ssh','--',s)
def probe(s):
 out=guest('LW_WALLPAPER_EVIDENCE="$HOME/wallpaper-fixed-evidence" LW_WALLPAPER_FIXTURE="$HOME/wallpaper-fixture-fixed" LW_WALLPAPER_MODE="$HOME/lewdware-fixed/wallpaper-test.lwmode" python3 ~/wallpaper-probe-fixed.py '+s)
 return [json.loads(x)['value'] for x in out.splitlines() if x.startswith('{')]
home=guest('printf %s "$HOME"').strip();vm('launch',home+'/lewdware-fixed/bin/lewdware-supervisor');time.sleep(1)
state=probe('status')[0]
if a.desktop=='gnome':probe('SetupStopShortcut');time.sleep(1)
if a.desktop=='sway':
 guest('SWAYSOCK=$(systemctl --user show-environment | sed -n "s/^SWAYSOCK=//p") swaymsg bindsym Shift+Escape exec "$HOME/lewdware-fixed/bin/lewdware-supervisor stop-immediately"')
# Native Wayland window focus is essential for compositor shortcut coverage.
if a.desktop=='kde':vm('launch','/usr/bin/env','QT_QPA_PLATFORM=wayland','konsole')
elif a.desktop=='gnome':vm('launch','/usr/bin/gnome-terminal')
elif a.desktop=='sway':vm('launch','/usr/bin/foot')
else:vm('launch','/usr/bin/xterm')
time.sleep(2);probe('start hold');time.sleep(3);vm('shot',str(ev/'shortcut-focused-before.png'));probe('status');vm('key','shift+esc');time.sleep(2)
state=probe('status')[0];vm('shot',str(ev/'shortcut-focused-after.png'))
try:assert state['session']['type']=='Idle',state
finally:probe('stop')
# The suite restores stock settings at the end, so compare to those here.
guest('cp ~/wallpaper-fixed-evidence/original.json ~/wallpaper-fixed-evidence/known.json')
probe('check');print('FOCUSED SHORTCUT PASSED',flush=True)
