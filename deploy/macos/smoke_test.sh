#!/usr/bin/env bash
# Installs a Lewdware .pkg and checks that what it installed runs on this Mac: the config app and
# the pack editor (gpui, so they need Metal, which the macOS test VMs don't have), and a session
# through the supervisor and the engine. Leaves screenshots and logs in OUT_DIR.
#
# Run from the repository root: deploy/macos/smoke_test.sh dist/lewdware_<version>_<arch>.pkg out
# CI runs it on an Apple Silicon runner after the arm64 build (.github/workflows/build.yml).
# It installs into /Applications and replaces Lewdware's config, so only run it on a test machine.
#
# Doesn't cover: permission prompts (Input Monitoring for the stop shortcut), which need a
# person to answer them; the macOS VM covers those (deploy/vm/).
set -uo pipefail

PKG="$1"
OUT="$2"
mkdir -p "$OUT"

APP=/Applications/Lewdware.app
EDITOR="/Applications/Lewdware Pack Editor.app"
SUPERVISOR="$APP/Contents/MacOS/lewdware-supervisor"
CONFIG="$HOME/Library/Application Support/lewdware/config.json"
LOGS="$HOME/Library/Logs/lewdware"
AGENT=com.lewdware.smoke-test-supervisor
failures=()

fail() {
  echo "FAIL: $1"
  failures+=("$1")
}

shot() {
  screencapture -x "$OUT/$1.png" || echo "screencapture failed (no Screen Recording permission?)"
}

# Whether one of the installed executables is running. Matched by path: macOS cuts process
# names to 16 characters, so `pgrep -x lewdware-pack-editor` never matches.
running() {
  pgrep -f "Contents/MacOS/$1( |\$)" >/dev/null
}

stop() {
  pkill -f "Contents/MacOS/$1( |\$)"
}

echo "== Install"
sudo installer -pkg "$PKG" -target / || { fail "installer"; exit 1; }
codesign --verify --deep "$APP" || fail "Lewdware.app's signature doesn't verify"
codesign --verify --deep "$EDITOR" || fail "the pack editor's signature doesn't verify"
ls -R "$APP/Contents" "$EDITOR/Contents" >"$OUT/bundles.txt"

echo "== Config app"
open "$APP"
sleep 30
shot config
if running lewdware; then
  echo "config app running"
else
  fail "the config app exited"
fi

echo "== Pack editor"
open "$EDITOR"
sleep 30
shot pack-editor
if running lewdware-pack-editor; then
  echo "pack editor running"
else
  fail "the pack editor exited"
fi

if grep -h "Unable to enumerate Metal devices" "$LOGS"/*.jsonl.* 2>/dev/null; then
  fail "gpui found no Metal device"
fi

osascript -e 'quit app "Lewdware Pack Editor"' -e 'quit app "Lewdware"' 2>/dev/null
sleep 5
stop lewdware-pack-editor
stop lewdware

echo "== Session"
# An empty pack and the theme gallery mode, which shows a window per theme.
python3 deploy/vm/make-empty-pack.py "$OUT/smoke.lwpack" || fail "couldn't make the test pack"
(cd dev-modes/theme-gallery && "$APP/Contents/MacOS/lw" mode build) || fail "couldn't build the test mode"
MODE="$(pwd)/dev-modes/theme-gallery/build/Theme gallery.lwmode"
python3 - "$CONFIG" "$OUT/smoke.lwpack" "$MODE" <<'EOF' || fail "couldn't write the config"
import json, os, sys
path, pack, mode = sys.argv[1:]
config = json.load(open(path)) if os.path.exists(path) else {}
config.update(pack_path=pack, mode={"File": {"path": mode}}, uploaded_modes=[mode])
os.makedirs(os.path.dirname(path), exist_ok=True)
json.dump(config, open(path, "w"), indent=2)
EOF

# Started by launchd from inside Lewdware.app, as a scheduled session would start it, so that
# it runs in the GUI session and as Lewdware.app.
if ! "$SUPERVISOR" status >/dev/null 2>&1; then
  cat >"/tmp/$AGENT.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>$AGENT</string>
<key>ProgramArguments</key><array><string>$SUPERVISOR</string></array>
<key>RunAtLoad</key><true/>
</dict></plist>
EOF
  launchctl bootstrap "gui/$(id -u)" "/tmp/$AGENT.plist"
  sleep 5
fi
"$SUPERVISOR" start || fail "the supervisor didn't start a session"
sleep 20
shot session
"$SUPERVISOR" status >"$OUT/status.txt" 2>&1
cat "$OUT/status.txt"
if grep -q "Running" "$OUT/status.txt" && running lewdware-engine; then
  echo "session running"
else
  fail "the session isn't running"
fi
if grep -h "falling back to software rendering" "$LOGS"/lewdware.jsonl.* 2>/dev/null; then
  fail "the engine found no GPU"
fi

"$SUPERVISOR" stop
sleep 5
if running lewdware-engine; then
  fail "the engine is still running after stop"
fi
shot stopped
launchctl bootout "gui/$(id -u)/$AGENT" 2>/dev/null
stop lewdware-supervisor

cp -R "$LOGS" "$OUT/logs" 2>/dev/null
ls ~/Library/Logs/DiagnosticReports 2>/dev/null | grep -i lewdware >"$OUT/crashes.txt" &&
  cp ~/Library/Logs/DiagnosticReports/*lewdware* "$OUT/" 2>/dev/null

if ((${#failures[@]})); then
  printf '\n%s failure(s):\n' "${#failures[@]}"
  printf '  %s\n' "${failures[@]}"
  exit 1
fi
echo "All checks passed"
