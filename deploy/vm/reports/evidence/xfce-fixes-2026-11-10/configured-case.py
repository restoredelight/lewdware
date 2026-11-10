import json,subprocess,shutil
from pathlib import Path
home=Path.home()
image=home/"wallpaper-fixture/Original ' 雪.png "
shutil.copyfile(home/'wallpaper-fixture/B.png',image)
def conf(key,kind,value):
 subprocess.run(['xfconf-query','-c','xfce4-desktop','-p',key,'--create','--type',kind,'--set',str(value)],check=True)
def listing():return subprocess.check_output(['xfconf-query','-c','xfce4-desktop','-lv'])
conf('/backdrop/single-workspace-mode','bool','true')
conf('/backdrop/single-workspace-number','int',2)
conf('/backdrop/screen0/monitorVirtual-1/workspace2/last-image','string',image)
conf('/backdrop/screen0/monitorVirtual-1/workspace2/image-style','int',4)
conf('/backdrop/screen0/monitorVirtual-1/workspace0/last-image','string','/untouched-workspace.png')
before=listing(); (home/'xfce-configured-before.txt').write_bytes(before)
def probe(*args):
 subprocess.run(['systemd-run','--user','--wait','--pipe','--collect',str(home/'wallpaper-probe-fixed'),*map(str,args)],check=True)
state=home/'xfce-configured-snapshot.json'
probe('capture',state)
snapshot=json.loads(state.read_text())
assert len(snapshot['entries'])==2,snapshot
assert all('/workspace2/' in p and v is not None for p,v in snapshot['entries'])
assert any(value==str(image) for _,value in snapshot['entries'])
probe('apply',state,home/'wallpaper-fixture/A.png')
probe('apply',state,home/'wallpaper-fixture/B.png')
probe('restore',state)
after=listing();(home/'xfce-configured-after.txt').write_bytes(after)
assert before==after,(before,after)
print('CONFIGURED_SHARED_WORKSPACE_EXACT_RESTORATION_PASSED; trailing-space and Unicode filename preserved')
