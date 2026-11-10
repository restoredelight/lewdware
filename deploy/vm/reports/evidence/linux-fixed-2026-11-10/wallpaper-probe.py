#!/usr/bin/env python3
"""Run inside a disposable Linux guest to record real wallpaper/session state.

Requires the synthetic fixture at ~/wallpaper-fixture and compiled mode at
~/wallpaper-test.lwmode. Commands: prepare, known, start SCENARIO, status,
check, stop, kill-engine, kill-both, permission on|off, restore-original.
Snapshots and an append-only JSONL record are saved under ~/wallpaper-evidence.
LW_WALLPAPER_EVIDENCE, LW_WALLPAPER_FIXTURE and LW_WALLPAPER_MODE override paths.
Supports GNOME, Cinnamon, MATE, Xfce, Plasma, and Sway with swaybg.
"""
import datetime
import json
import os
import re
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import time

HOME = Path.home()
EVIDENCE = Path(os.environ.get('LW_WALLPAPER_EVIDENCE', HOME / 'wallpaper-evidence'))
FIXTURE = Path(os.environ.get('LW_WALLPAPER_FIXTURE', HOME / 'wallpaper-fixture'))
MODE = Path(os.environ.get('LW_WALLPAPER_MODE', HOME / 'wallpaper-test.lwmode'))
CONFIG = HOME / '.config/lewdware/config.json'
os.environ.setdefault('DBUS_SESSION_BUS_ADDRESS', f'unix:path=/run/user/{os.getuid()}/bus')


def run(*args):
    return subprocess.check_output(args, text=True).removesuffix('\n')


def record(kind, value):
    event = dict(time=datetime.datetime.now(datetime.timezone.utc).isoformat(), kind=kind, value=value)
    EVIDENCE.mkdir(exist_ok=True)
    with (EVIDENCE / 'events.jsonl').open('a') as f:
        f.write(json.dumps(event, ensure_ascii=False) + '\n')
    print(json.dumps(event, ensure_ascii=False), flush=True)


def request(kind, **extra):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(15)
        s.connect('\0lewdware-supervisor.sock')
        s.sendall((json.dumps(dict(type=kind, **extra)) + '\n').encode())
        reply = json.loads(s.makefile().readline())
    record('ipc', reply)
    if reply['type'] == 'Error':
        raise RuntimeError(reply['message'])
    return reply


def plasma(script):
    return run('dbus-send', '--session', '--print-reply=literal', '--reply-timeout=5000',
               '--dest=org.kde.plasmashell', '/PlasmaShell',
               'org.kde.PlasmaShell.evaluateScript', 'string:' + script)


def backend():
    # SSH does not inherit the desktop environment. Query the graphical user manager.
    env = run('systemctl', '--user', 'show-environment')
    desktop = next((x.partition('=')[2] for x in env.splitlines() if x.startswith('XDG_CURRENT_DESKTOP=')), '').lower()
    if 'xfce' in desktop:
        return 'xfce'
    if 'kde' in desktop or 'plasma' in desktop:
        return 'kde'
    for candidate in ('cinnamon', 'mate', 'gnome', 'sway'):
        if candidate in desktop:
            return candidate
    raise RuntimeError(f'Unsupported user-manager desktop: {desktop!r}')


DCONF_KEYS = {
    'cinnamon': ('/org/cinnamon/desktop/background/picture-uri',
                 '/org/cinnamon/desktop/background/picture-options'),
    'mate': ('/org/mate/desktop/background/picture-filename',
             '/org/mate/desktop/background/picture-options'),
}
GNOME_KEYS = ('picture-uri', 'picture-uri-dark', 'picture-options')


def swaybg_processes():
    found = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            if (entry / 'comm').read_text().strip() != 'swaybg':
                continue
            argv = (entry / 'cmdline').read_bytes().rstrip(b'\0').split(b'\0')
            if argv != [b'']:
                found.append((int(entry.name), [x.decode() for x in argv]))
        except (OSError, RuntimeError):
            pass
    return found


def settings():
    desktop = backend()
    if desktop == 'xfce':
        keys = run('xfconf-query', '-c', 'xfce4-desktop', '-l').splitlines()
        return {k: run('xfconf-query', '-c', 'xfce4-desktop', '-p', k)
                for k in sorted(keys) if k.endswith(('last-image', 'image-style'))}
    if desktop == 'gnome':
        return {k: run('gsettings', 'get', 'org.gnome.desktop.background', k)
                for k in GNOME_KEYS}
    if desktop in DCONF_KEYS:
        return {k: run('dconf', 'read', k) or None for k in DCONF_KEYS[desktop]}
    if desktop == 'sway':
        return sorted(argv for _, argv in swaybg_processes())
    return json.loads(plasma('''var out=[]; var ds=desktops();
for(var i=0;i<ds.length;i++){var d=ds[i]; var p=d.wallpaperPlugin;
d.currentConfigGroup=["Wallpaper",p,"General"];
out.push({id:d.id,plugin:p,image:d.readConfig("Image"),fill_mode:d.readConfig("FillMode")});}
print(JSON.stringify(out));'''))


def apply_settings(saved):
    desktop = backend()
    if desktop == 'xfce':
        current = settings()
        for key in current.keys() - saved.keys():
            run('xfconf-query', '-c', 'xfce4-desktop', '-p', key, '-r')
        for key, value in saved.items():
            if key not in current:
                run('xfconf-query', '-c', 'xfce4-desktop', '-p', key, '-n',
                    '-t', 'int' if key.endswith('image-style') else 'string', '-s', value)
            else:
                run('xfconf-query', '-c', 'xfce4-desktop', '-p', key, '-s', value)
    elif desktop == 'gnome':
        for key, value in saved.items():
            run('gsettings', 'set', 'org.gnome.desktop.background', key, value)
    elif desktop in DCONF_KEYS:
        for key, value in saved.items():
            if value is None:
                run('dconf', 'reset', key)
            else:
                run('dconf', 'write', key, value)
    elif desktop == 'sway':
        env = dict(os.environ)
        env.update(line.split('=', 1) for line in run('systemctl', '--user', 'show-environment').splitlines() if '=' in line)
        for pid, _ in swaybg_processes():
            os.kill(pid, signal.SIGTERM)
        for argv in saved:
            subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                             start_new_session=True)
        time.sleep(0.4)
    else:
        plasma('''var saved=%s; var ds=desktops();
for(var i=0;i<ds.length;i++){var d=ds[i];
for(var j=0;j<saved.length;j++){var e=saved[j]; if(d.id!=e.id)continue;
d.wallpaperPlugin=e.plugin;d.currentConfigGroup=["Wallpaper",e.plugin,"General"];
d.writeConfig("Image",e.image);d.writeConfig("FillMode",e.fill_mode);}}''' % json.dumps(saved))


def exact_pid(name):
    found = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / 'exe').resolve(strict=True).name == name:
                found.append(int(entry.name))
        except (OSError, RuntimeError):
            pass
    if len(found) != 1:
        raise RuntimeError(f'Expected one exact {name} process; got {found}')
    return found[0]


def main():
    command = sys.argv[1]
    for line in run('systemctl', '--user', 'show-environment').splitlines():
        key, separator, value = line.partition('=')
        if separator and key in ('DISPLAY', 'XAUTHORITY', 'WAYLAND_DISPLAY', 'SWAYSOCK',
                                 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS'):
            os.environ[key] = value
    EVIDENCE.mkdir(exist_ok=True)
    if command == 'prepare':
        original = EVIDENCE / 'original.json'
        with original.open('x') as f:
            json.dump(settings(), f, indent=2)
        record('original', settings())
        if CONFIG.exists():
            shutil.copyfile(CONFIG, EVIDENCE / 'original-config.json')
            c = json.loads(CONFIG.read_text())
        else:
            c = dict(uploaded_modes=[], theme='native', appearance='auto',
                     stop_button=dict(name='Escape', code='Escape', modifiers=dict(alt=False, ctrl=False, shift=True, meta=False)),
                     disabled_monitors=[], monitor_regions={},
                     permissions=dict(set_wallpaper=True, open_links=False, send_notifications=False),
                     wallpaper=dict(restore=dict(kind='original')),
                     volume=dict(video=1.0, audio=1.0), audio_device=None,
                     schedule=dict(enabled=False, rules=[], quiet_hours=[], grace_notification=True, cooldown_minutes=30))
        c.update(pack_path=str(FIXTURE / 'wallpaper.lwpack'), uploaded_modes=[str(MODE)],
                 mode={'File': {'path': str(MODE)}}, mode_options=[])
        c['permissions']['set_wallpaper'] = True
        c['wallpaper'] = {'restore': {'kind': 'original'}}
        c['schedule']['enabled'] = False
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        CONFIG.write_text(json.dumps(c))
    elif command == 'known':
        # Exercise spaces, an apostrophe, and non-ASCII in the restore path.
        image = FIXTURE / "Original ' 雪.png"
        shutil.copyfile(FIXTURE / 'B.png', image)
        saved = settings()
        if backend() == 'xfce':
            outputs = run('xrandr', '--query')
            assert outputs.startswith('Screen 0:'), outputs
            connectors = [line.split()[0] for line in outputs.splitlines()
                          if ' connected ' in line and re.search(r'\b\d+x\d+[+-]\d+[+-]\d+', line)]
            assert connectors, outputs
            count = int(run('xprop', '-root', '_NET_NUMBER_OF_DESKTOPS').split('=')[1])
            assert 0 < count <= 1024, count
            keys = run('xfconf-query', '-c', 'xfce4-desktop', '-l').splitlines()
            workspaces = range(count)
            single = '/backdrop/single-workspace-mode'
            number = '/backdrop/single-workspace-number'
            if single in keys and run('xfconf-query', '-c', 'xfce4-desktop', '-p', single) == 'true':
                workspaces = [int(run('xfconf-query', '-c', 'xfce4-desktop', '-p', number)) if number in keys else 0]
            for connector in connectors:
                for workspace in workspaces:
                    prefix = f'/backdrop/screen0/monitor{connector}/workspace{workspace}'
                    saved[prefix + '/last-image'] = str(image)
                    saved[prefix + '/image-style'] = '3'
        elif backend() == 'gnome':
            saved = {k: repr('scaled' if k == 'picture-options' else image.as_uri()) for k in saved}
        elif backend() in DCONF_KEYS:
            picture, options = DCONF_KEYS[backend()]
            saved = {picture: repr(str(image) if backend() == 'mate' else image.as_uri()), options: repr('scaled')}
        elif backend() == 'sway':
            saved = [['swaybg', '-i', str(image), '-m', 'fit']]
        else:
            for desktop in saved:
                desktop.update(plugin='org.kde.image', image=image.as_uri(), fill_mode='2')
        apply_settings(saved)
        (EVIDENCE / 'known.json').write_text(json.dumps(settings(), indent=2))
        record('known', settings())
    elif command == 'start':
        c = json.loads(CONFIG.read_text())
        c['mode_options'] = [[c['mode'], {'scenario': sys.argv[2]}]]
        CONFIG.write_text(json.dumps(c))
        reply = request('StartSession', mode_path=str(MODE), dev=False, replace=False)
        assert reply['type'] == 'Ok', reply
    elif command == 'status':
        request('Status')
        record('settings', settings())
        path = HOME / '.local/share/lewdware/wallpaper-snapshot.json'
        record('snapshot', json.loads(path.read_text()) if path.exists() else None)
    elif command == 'check':
        actual = settings()
        expected = json.loads((EVIDENCE / 'known.json').read_text())
        record('restoration', dict(actual=actual, expected=expected, passed=actual == expected))
        assert actual == expected
    elif command == 'stop':
        request('StopImmediately')
    elif command == 'crash-stop':
        stop = sys.argv[2]
        assert stop in ('StopSession', 'StopImmediately')
        state = request('Status')
        assert state['session']['type'] == 'Running', state
        pid = state['session']['pid']
        assert Path(f'/proc/{pid}/exe').resolve().name == 'lewdware-engine'
        record('kill-engine', pid)
        os.kill(pid, signal.SIGKILL)
        deadline = time.monotonic() + 10
        while True:
            state = request('Status')
            if state['session']['type'] == 'RestartPending':
                break
            if state['session']['type'] == 'Running':
                assert state['session']['pid'] == pid, state
            assert time.monotonic() < deadline, state
            time.sleep(0.02)
        request(stop)
        state = request('Status')
        assert state['session']['type'] == 'Idle', state
    elif command in ('kill-engine', 'kill-both'):
        state = request('Status')
        pid = state['session']['pid']
        assert Path(f'/proc/{pid}/exe').resolve().name == 'lewdware-engine'
        if command == 'kill-both':
            supervisor = exact_pid('lewdware-supervisor')
            record('kill-supervisor', supervisor)
            os.kill(supervisor, signal.SIGKILL)
        record('kill-engine', pid)
        os.kill(pid, signal.SIGKILL)
    elif command == 'permission':
        c = json.loads(CONFIG.read_text())
        c['permissions']['set_wallpaper'] = sys.argv[2] == 'on'
        CONFIG.write_text(json.dumps(c))
    elif command == 'restore-original':
        apply_settings(json.loads((EVIDENCE / 'original.json').read_text()))
        record('original-restored', settings())
    else:
        request(command)


if __name__ == '__main__':
    main()
