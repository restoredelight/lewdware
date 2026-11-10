import json, os, socket, subprocess, sys
from pathlib import Path
os.environ['DBUS_SESSION_BUS_ADDRESS']='unix:path=/run/user/1000/bus'
root=Path.home(); cfg=root/'.config/lewdware/config.json'
def request(kind, **extra):
 s=socket.socket(socket.AF_UNIX); s.settimeout(15); s.connect('\0lewdware-supervisor.sock'); s.sendall((json.dumps(dict(type=kind,**extra))+'\n').encode()); r=json.loads(s.makefile().readline()); print(json.dumps(r)); return r
def settings():
 return {k:subprocess.check_output(['gsettings','get','org.gnome.desktop.background',k],text=True).strip() for k in ['picture-uri','picture-uri-dark','picture-options']}
cmd=sys.argv[1]
if cmd=='prepare':
 (root/'wallpaper-original.json').write_text(json.dumps(settings()))
 (root/'wallpaper-config-original.json').write_bytes(cfg.read_bytes())
 c=json.loads(cfg.read_text()); c.update(pack_path=str(root/'wallpaper-fixture/wallpaper.lwpack'),mode={'File':{'path':str(root/'wallpaper-test.lwmode')}},mode_options=[]); cfg.write_text(json.dumps(c))
 for key,label in [('picture-uri','A'),('picture-uri-dark','B')]:
  subprocess.run(['gsettings','set','org.gnome.desktop.background',key,(root/f'wallpaper-fixture/{label}.png').as_uri()],check=True)
 subprocess.run(['gsettings','set','org.gnome.desktop.background','picture-options','scaled'],check=True)
 (root/'wallpaper-known.json').write_text(json.dumps(settings()))
elif cmd=='start':
 c=json.loads(cfg.read_text()); c['mode_options']=[[c['mode'], {'scenario':sys.argv[2]}]]
 # Option values are a per-mode map; resolved below after checking schema.
 cfg.write_text(json.dumps(c)); request('StartSession',mode_path=str(root/'wallpaper-test.lwmode'),dev=False,replace=False)
elif cmd=='status': request('Status'); print(json.dumps(settings())); p=root/'.local/share/lewdware/wallpaper-snapshot.json'; print('snapshot',p.read_text() if p.exists() else 'ABSENT')
elif cmd=='stop': request('StopImmediately')
elif cmd in ['kill-engine','kill-both']:
 import signal
 state=request('Status'); engine=state['session']['pid']
 assert Path(f'/proc/{engine}/exe').resolve().name=='lewdware-engine'
 if cmd=='kill-both':
  supervisors=[int(p.name) for p in Path('/proc').iterdir() if p.name.isdigit() and p.joinpath('comm').exists() and p.joinpath('comm').read_text().strip()=='lewdware-superv']
  if not supervisors:
   supervisors=[]
   for p in Path('/proc').iterdir():
    try:
     if p.name.isdigit() and p.joinpath('exe').resolve().name=='lewdware-supervisor': supervisors.append(int(p.name))
    except (OSError,RuntimeError): pass
  assert len(supervisors)==1,supervisors
  print('killing exact guest supervisor',supervisors[0]); os.kill(supervisors[0],signal.SIGKILL)
 print('killing exact guest engine',engine); os.kill(engine,signal.SIGKILL)
elif cmd=='permission':
 c=json.loads(cfg.read_text()); c['permissions']['set_wallpaper']=sys.argv[2]=='on'; cfg.write_text(json.dumps(c))
elif cmd=='check':
 actual=settings(); expected=json.loads((root/'wallpaper-known.json').read_text()); print(json.dumps(actual)); assert actual==expected,(actual,expected)
else: request(cmd)
