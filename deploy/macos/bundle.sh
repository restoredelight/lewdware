#!/bin/bash
# deploy/macos/bundle.sh
# Helpers for assembling .app bundles by hand, sourced by build_installer.sh. The gpui apps
# have no bundler, so this writes what Tauri's used to: the Info.plist, the icon, and the
# non-system dylibs under Contents/Frameworks.

# make_app_bundle APP_DIR EXECUTABLE_SRC EXECUTABLE_NAME DISPLAY_NAME BUNDLE_ID VERSION [background]
#
# `background` makes an agent app (LSUIElement): no Dock icon or menu bar of its own.
#
# The bundle id matters beyond the plist: macOS keys privacy permissions (Input Monitoring,
# Accessibility) on it, so keep it stable across releases. The config app's is `com.lewdware`, as
# the Tauri build's was, which keeps existing grants.
make_app_bundle() {
  local app="$1" exe_src="$2" exe_name="$3" display_name="$4" bundle_id="$5" version="$6"
  local agent=""
  [[ "${7:-}" == background ]] && agent="<key>LSUIElement</key><true/>"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
  cp "$exe_src" "$app/Contents/MacOS/$exe_name"
  chmod +x "$app/Contents/MacOS/$exe_name"
  cp "assets/icons/icon.icns" "$app/Contents/Resources/icon.icns"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>${display_name}</string>
  <key>CFBundleName</key><string>${display_name}</string>
  <key>CFBundleExecutable</key><string>${exe_name}</string>
  <key>CFBundleIconFile</key><string>icon.icns</string>
  <key>CFBundleIdentifier</key><string>${bundle_id}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>10.15.7</string>
  <key>NSHighResolutionCapable</key><true/>
  ${agent}
</dict>
</plist>
PLIST
}

# bundle_dylibs APP_DIR BINARY
#
# Recursively copies every non-system dylib BINARY links into APP_DIR/Contents/Frameworks and
# relinks it there. Handles transitive deps automatically.
#
# References go through @loader_path, relative to whichever file loads the library, not
# @executable_path: the shared core in Lewdware.app's Frameworks is also loaded by the pack
# editor's launcher, whose executable is in another bundle.
bundle_dylibs() {
  local app="$1" target="$2"
  local frameworks="$app/Contents/Frameworks"
  local from_target="@loader_path/../Frameworks"
  [[ "$(dirname "$target")" == "$frameworks" ]] && from_target="@loader_path"
  while IFS= read -r dep; do
    case "$dep" in
      /usr/lib/* | /System/* | @* | "") continue ;;
    esac
    local lib_name
    lib_name=$(basename "$dep")
    # A library's first line in otool -L is its own install name, not a dependency.
    [[ "$lib_name" == "$(basename "$target")" ]] && continue
    local staged="$frameworks/$lib_name"
    install_name_tool -change "$dep" "$from_target/$lib_name" "$target"
    if [ ! -f "$staged" ]; then
      echo "   Bundling $dep"
      cp "$dep" "$staged"
      chmod 755 "$staged"
      install_name_tool -id "@loader_path/$lib_name" "$staged"
      bundle_dylibs "$app" "$staged"
    fi
  done < <(otool -L "$target" | tail -n +2 | awk '{print $1}')
}

# sign_app APP_DIR
#
# Ad-hoc signs the bundled dylibs, every executable in Contents/MacOS, and then the bundle.
# Apple Silicon refuses to run unsigned code, and install_name_tool invalidates any signature.
sign_app() {
  local app="$1"
  find "$app/Contents/Frameworks" -type f -name "*.dylib" -exec codesign --force --sign - {} \;
  find "$app/Contents/MacOS" -type f -perm -u+x -exec codesign --force --sign - {} \;
  codesign --force --sign - "$app"
}
