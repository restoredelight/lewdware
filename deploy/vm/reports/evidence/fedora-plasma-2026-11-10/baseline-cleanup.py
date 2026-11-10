import json,subprocess,shutil
from pathlib import Path
import runpy
p=runpy.run_path(str(Path.home()/'wallpaper-probe.py'))
p['apply_settings'](json.loads((Path.home()/'wallpaper-evidence/original.json').read_text()))
assert not (Path.home()/'.local/share/lewdware/wallpaper-snapshot.json').exists()
cfg=Path.home()/'.config/lewdware/config.json'; c=json.loads(cfg.read_text())
c['uploaded_modes']=[str(Path.home()/'wallpaper-test.lwmode')]
c['mode_options']=[[c['mode'],{'scenario':'hold'}]]
c['permissions']['set_wallpaper']=True
cfg.write_text(json.dumps(c))
(Path.home()/'.config/autostart/vmctl-fingerprint.desktop').unlink(missing_ok=True)
print('Baseline app and fixture configured; original desktop restored; recovery snapshot absent.')
