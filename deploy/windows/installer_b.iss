; installer_b.iss
; Inno Setup Script for Lewdware Pack Editor (Installer B)

[Setup]
AppName=Lewdware Pack Editor
AppVersion={#AppVersion}
AppPublisher=restoredelight
AppPublisherURL=https://lewdware.net
AppSupportURL=https://lewdware.net
AppUpdatesURL=https://lewdware.net/download/pack-editor
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Lewdware Pack Editor
DefaultGroupName=Lewdware
OutputDir=..\..\dist
OutputBaseFilename=lewdware-pack-editor_{#AppVersion}_x86_64
Compression=lzma2/max
SolidCompression=yes
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
SetupIconFile=..\..\assets\icons\icon.ico
UninstallDisplayIcon={app}\lewdware-pack-editor.exe
UninstallDisplayName=Lewdware Pack Editor
; The binaries, sidecars and vc_redist are x64-only.
ArchitecturesAllowed=x64compatible
LicenseFile=..\..\LICENSE
; Close a running editor (through the Restart Manager) rather than fail on its locked files.
CloseApplications=yes
RestartApplications=no

; FFmpeg is linked statically now. Clear out the DLLs earlier versions installed beside the exe.
[InstallDelete]
Type: files; Name: "{app}\avcodec-*.dll"
Type: files; Name: "{app}\avformat-*.dll"
Type: files; Name: "{app}\avutil-*.dll"
Type: files; Name: "{app}\avfilter-*.dll"
Type: files; Name: "{app}\avdevice-*.dll"
Type: files; Name: "{app}\swscale-*.dll"
Type: files; Name: "{app}\swresample-*.dll"
Type: files; Name: "{app}\dav1d.dll"

[Files]
Source: "..\..\target\release\lewdware-pack-editor.exe"; DestDir: "{app}"; Flags: ignoreversion
; ffmpeg, ffprobe and avifenc sidecars. The editor looks for them in binaries\ beside itself
; (pack-editor-gpui/src/main.rs, `sidecar_dirs`).
Source: "..\..\pack-editor-gpui\binaries\lewdware-ffmpeg.exe"; DestDir: "{app}\binaries"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\binaries\lewdware-ffprobe.exe"; DestDir: "{app}\binaries"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\binaries\lewdware-avifenc.exe"; DestDir: "{app}\binaries"; Flags: ignoreversion
; Licences: MIT for the editor, and the GPLv3 that covers the ffmpeg sidecars
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\COPYING.GPLv3"; DestDir: "{app}"; DestName: "COPYING.GPLv3.txt"; Flags: ignoreversion
; Visual C++ Redistributable (extracted to temp and deleted after install)
Source: "..\..\build\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{group}\Lewdware Pack Editor"; Filename: "{app}\lewdware-pack-editor.exe"
Name: "{userdesktop}\Lewdware Pack Editor"; Filename: "{app}\lewdware-pack-editor.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Run]
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing Visual C++ Redistributable..."
Filename: "{app}\lewdware-pack-editor.exe"; Description: "{cm:LaunchProgram,Lewdware Pack Editor}"; Flags: nowait postinstall skipifsilent
