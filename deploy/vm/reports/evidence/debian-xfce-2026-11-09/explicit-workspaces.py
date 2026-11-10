import os,subprocess
os.environ['DBUS_SESSION_BUS_ADDRESS']='unix:path=/run/user/1000/bus'
for w in range(4):
 for leaf,kind,value in [('last-image','string','/usr/share/images/desktop-base/default'),('image-style','int','3')]:
  key=f'/backdrop/screen0/monitorVirtual-1/workspace{w}/{leaf}'
  base=['xfconf-query','-c','xfce4-desktop','-p',key]
  exists=subprocess.run(base,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
  subprocess.run(base+([] if exists else ['-n','-t',kind])+['-s',value],check=True)
