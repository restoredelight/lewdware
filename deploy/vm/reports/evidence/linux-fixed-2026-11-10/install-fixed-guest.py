import argparse,pathlib,subprocess
p=argparse.ArgumentParser();p.add_argument('vm');a=p.parse_args()
base=pathlib.Path('/home/restoredelight/projects/lewdware')
def vm(*args):subprocess.run([str(base/'deploy/vm/vmctl'),'--vm',a.vm,*args],check=True)
vm('ready')
vm('push','/home/restoredelight/.cache/lewdware-vm-build/lewdware-fixed.tgz','/tmp/lewdware-fixed.tgz')
vm('push','/home/restoredelight/.cache/lewdware-vm-build/fixed-source.tgz','/tmp/fixed-source.tgz')
vm('push','/tmp/wallpaper-fixture-fixed.tgz','/tmp/wallpaper-fixture-fixed.tgz')
vm('push',str(base/'deploy/vm/wallpaper-probe.py'),'/tmp/wallpaper-probe-fixed.py')
vm('ssh','--', '''set -eu
mkdir -p ~/lewdware-fixed ~/lewdware-fixed-source ~/wallpaper-fixture-fixed ~/.local/share/applications
tar -xzf /tmp/lewdware-fixed.tgz -C ~/lewdware-fixed
tar -xzf /tmp/fixed-source.tgz -C ~/lewdware-fixed-source
tar -xzf /tmp/wallpaper-fixture-fixed.tgz -C ~/wallpaper-fixture-fixed
cp /tmp/wallpaper-probe-fixed.py ~/wallpaper-probe-fixed.py
(cd ~/lewdware-fixed && sha256sum -c SHA256SUMS)
for bin in ~/lewdware-fixed/bin/*; do
  if ldd "$bin" | grep 'not found'; then exit 1; fi
done
if [ -d ~/.local/share/lewdware/logs ]; then mv ~/.local/share/lewdware/logs ~/wallpaper-fixed-baseline-logs; fi
cat > ~/.local/share/applications/com.lewdware.supervisor.desktop <<EOF
[Desktop Entry]
Name=Lewdware
Comment=Lewdware fixed core diagnostic build
Exec=$HOME/lewdware-fixed/bin/lewdware-supervisor
Terminal=false
Type=Application
NoDisplay=true
EOF
python3 - <<'END'
from pathlib import Path
import os,signal,time,hashlib,json
source=Path.home()/'lewdware-fixed-source'
manifest=json.loads((source/'vm-source-manifest.json').read_text())
assert all(hashlib.sha256((source/name).read_bytes()).hexdigest()==digest for name,digest in manifest['files'].items())
print('SOURCE_INPUTS_VERIFIED',len(manifest['files']),manifest['head'])
owned=[]
for p in Path('/proc').iterdir():
 if not p.name.isdigit():continue
 try:
  if p.stat().st_uid!=os.getuid():continue
  exe=(p/'exe').resolve(strict=True)
  if exe.name in ('lewdware-engine','lewdware-supervisor'):owned.append((int(p.name),exe))
 except (OSError,RuntimeError):pass
assert not any(exe.name=='lewdware-engine' for _,exe in owned),owned
for pid,exe in owned:
 assert str(exe).startswith(('/usr/lib/lewdware/','/home/')),exe
 os.kill(pid,signal.SIGTERM)
for _ in range(50):
 if all(not Path(f'/proc/{pid}').exists() for pid,_ in owned):break
 time.sleep(0.1)
else:raise RuntimeError('Existing guest supervisor did not exit')
END
systemctl --user show-environment
cat /etc/os-release
''')
