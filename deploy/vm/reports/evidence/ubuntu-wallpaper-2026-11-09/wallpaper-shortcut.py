import subprocess,time,pathlib
base=pathlib.Path('/home/restoredelight/projects/lewdware'); evidence=base/'deploy/vm/reports/evidence/ubuntu-wallpaper-2026-11-09'
def vm(*args):
 r=subprocess.run([str(base/'deploy/vm/vmctl'),'--vm','ubuntu-24.04',*args],capture_output=True,text=True); print(r.stdout,r.stderr,flush=True)
 with (evidence/'transcript.txt').open('a') as f: f.write(str(args)+'\n'+r.stdout+r.stderr)
 if r.returncode: raise RuntimeError(r.returncode)
 return r.stdout
def guest(s): return vm('ssh','--',s)
vm('launch','/usr/lib/lewdware/lewdware-supervisor')
vm('key','ctrl+alt+t'); time.sleep(2)
guest('python3 ~/wallpaper-guest.py start hold'); time.sleep(2)
vm('key','shift+esc'); time.sleep(1); guest('python3 ~/wallpaper-guest.py status'); vm('shot',str(evidence/'shortcut-native-wayland-fails.png'))
vm('click','110','180'); vm('key','shift+esc'); time.sleep(1); guest('python3 ~/wallpaper-guest.py status; python3 ~/wallpaper-guest.py check'); vm('shot',str(evidence/'shortcut-xwayland-passes.png'))
guest('journalctl --user -u vmctl-launch-1794260200252041053.service --no-pager -n 45')
guest('export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; gdbus call --system --dest org.freedesktop.Accounts --object-path /org/freedesktop/Accounts/User1000 --method org.freedesktop.Accounts.User.SetXSession ubuntu')
