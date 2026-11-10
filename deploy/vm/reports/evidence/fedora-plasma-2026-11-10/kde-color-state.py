import json,runpy
from pathlib import Path
p=runpy.run_path(str(Path.home()/'wallpaper-probe.py'))
p['plasma']('''var ds=desktops(); for(var i=0;i<ds.length;i++) {var d=ds[i];d.wallpaperPlugin="org.kde.color";d.currentConfigGroup=["Wallpaper","org.kde.color","General"];d.writeConfig("Color","#336699");}''')
state=p['settings']()
(Path.home()/'wallpaper-evidence/color-known.json').write_text(json.dumps(state,indent=2))
(Path.home()/'wallpaper-evidence/known.json').write_text(json.dumps(state,indent=2))
p['record']('color-known',state)
