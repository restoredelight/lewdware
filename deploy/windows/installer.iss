; installer.iss
; Inno Setup Script for Lewdware: the config app, the pack editor, the supervisor, the engine and
; the lw CLI.
;
; Every app but lw is in lewdware_core.dll; each .exe below but lw.exe is a small launcher that
; loads it (core\, launcher\). The launchers are built as launcher-<app>.exe and installed under
; their apps' names.

[Setup]
AppName=Lewdware
AppVersion={#AppVersion}
AppPublisher=restoredelight
AppPublisherURL=https://lewdware.net
AppSupportURL=https://lewdware.net
AppUpdatesURL=https://lewdware.net/download/
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Lewdware
DefaultGroupName=Lewdware
OutputDir=..\..\dist
OutputBaseFilename=lewdware_{#AppVersion}_x86_64
Compression=lzma2/max
SolidCompression=yes
ChangesEnvironment=yes
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
SetupIconFile=..\..\assets\icons\icon.ico
; Without this the Apps & features entry has no icon next to it.
UninstallDisplayIcon={app}\lewdware.exe
UninstallDisplayName=Lewdware
; The binaries and the vc_redist are x64-only.
ArchitecturesAllowed=x64compatible
LicenseFile=..\..\LICENSE
; StopLewdware (see [Code]) shuts everything down in PrepareToInstall, which runs *before* this
; check - so it normally finds nothing. Left enabled for what StopLewdware deliberately leaves
; running: the pack editor, which may have unsaved work (the Restart Manager asks before closing
; it), and a stray lw.exe running out of {app}.
CloseApplications=yes
; ...but don't let the Restart Manager relaunch the supervisor behind the user's back once the
; install finishes. Starting a session is the user's call.
RestartApplications=no

; FFmpeg is linked statically now. Clear out the DLLs earlier versions installed beside the exe.
; (The pack editor's old separate install, which had its own, is uninstalled in PrepareToInstall.)
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
; Licences. MIT's own terms require its notice to accompany copies of the software (it is also
; shown during install, above); the LGPL requires its text to accompany the FFmpeg statically
; linked into the core; the OFL requires the same of the fonts the apps embed.
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\lewdware\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; DestName: "THIRD_PARTY_LICENSES-pack-editor.md"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\COPYING.LGPLv2.1"; DestDir: "{app}"; DestName: "COPYING.LGPLv2.1.txt"; Flags: ignoreversion
Source: "..\..\assets\fonts\licenses\*"; DestDir: "{app}\fonts"; Flags: ignoreversion
; The shared core: every app but lw
Source: "..\..\target\release\lewdware_core.dll"; DestDir: "{app}"; Flags: ignoreversion
; The config app (user-facing entry point)
Source: "..\..\target\release\launcher-config.exe"; DestDir: "{app}"; DestName: "lewdware.exe"; Flags: ignoreversion
; The pack editor, and its image encoder, which it looks for in binaries\ beside the core
; (pack-editor-gpui/src/lib.rs, `sidecar_dirs`)
Source: "..\..\target\release\launcher-pack-editor.exe"; DestDir: "{app}"; DestName: "lewdware-pack-editor.exe"; Flags: ignoreversion
Source: "..\..\pack-editor-gpui\binaries\lewdware-avifenc.exe"; DestDir: "{app}\binaries"; Flags: ignoreversion
; Engine (internal, launched by the supervisor)
Source: "..\..\target\release\launcher-engine.exe"; DestDir: "{app}"; DestName: "lewdware-engine.exe"; Flags: ignoreversion
; Supervisor (internal, owns session lifecycle and scheduling)
Source: "..\..\target\release\launcher-supervisor.exe"; DestDir: "{app}"; DestName: "lewdware-supervisor.exe"; Flags: ignoreversion
; CLI
Source: "..\..\target\release\lw.exe"; DestDir: "{app}"; Flags: ignoreversion
; Visual C++ Redistributable (extracted to temp and deleted after install)
Source: "..\..\build\win-stage\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall

; Each with the AppUserModelID its app sets on itself (launcher/src/bin/), so a pinned shortcut and
; the running app share one taskbar button.
[Icons]
Name: "{group}\Lewdware"; Filename: "{app}\lewdware.exe"; AppUserModelID: "com.lewdware.config"
Name: "{group}\Lewdware Pack Editor"; Filename: "{app}\lewdware-pack-editor.exe"; AppUserModelID: "com.lewdware.pack-editor"
Name: "{userdesktop}\Lewdware"; Filename: "{app}\lewdware.exe"; AppUserModelID: "com.lewdware.config"; Tasks: desktopicon
Name: "{userdesktop}\Lewdware Pack Editor"; Filename: "{app}\lewdware-pack-editor.exe"; AppUserModelID: "com.lewdware.pack-editor"; Tasks: desktopicon

[Run]
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing Visual C++ Redistributable..."

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "addtopath"; Description: "Add 'lw' CLI to the user PATH environment variable"; GroupDescription: "Environment Setup:"

[Registry]
; Update the User PATH variable to expose 'lw' CLI
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Flags: preservestringtype; Tasks: addtopath; Check: NeedsAddPath

[Code]
const
  WM_SETTINGCHANGE = $1A;
  SMTO_ABORTIFHUNG = 2;

function SendMessageTimeoutW(hWnd: HWND; Msg: Cardinal; wParam: Longint; lParam: String;
  fuFlags: Cardinal; uTimeout: Cardinal; var lpdwResult: Longint): Longint;
  external 'SendMessageTimeoutW@user32.dll stdcall';

// Shut down anything that would otherwise hold {app}'s binaries open during an upgrade or
// uninstall. Called from PrepareToInstall (which runs before Setup's own file-in-use check)
// and from the uninstaller before any file is removed.
procedure StopLewdware();
var
  Supervisor: String;
  ResultCode: Integer;
  I: Integer;
  Images: array[0..2] of String;
begin
  Supervisor := ExpandConstant('{app}\lewdware-supervisor.exe');

  // Ask the supervisor to end a live session first, and give it a moment. The engine tears down
  // its windows and restores the desktop wallpaper on its way out; going straight to taskkill
  // would leave the user staring at a wallpaper the pack set. If no daemon is reachable this
  // just exits non-zero, which is fine - nothing to stop.
  if FileExists(Supervisor) then
  begin
    Exec(Supervisor, 'stop', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    Sleep(2000);
  end;

  // Then make sure nothing is still holding the files. There is no IPC request that shuts the
  // daemon itself down (only StopSession), so the supervisor has to be killed outright. lw.exe
  // is deliberately not in this list: it's a short-lived CLI, and killing a running
  // `lw mode build` mid-write is worse than the rare locked-file error CloseApplications
  // already covers.
  Images[0] := 'lewdware-engine.exe';
  Images[1] := 'lewdware-supervisor.exe';
  Images[2] := 'lewdware.exe';
  for I := 0 to 2 do
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM ' + Images[I], '',
      SW_HIDE, ewWaitUntilTerminated, ResultCode);

  Sleep(500);
end;

// The pack editor used to have an installer of its own, into {localappdata}\Programs\Lewdware
// Pack Editor. It is part of this install now, so remove that one, quietly, rather than leave two
// copies and two Start menu entries. Its uninstaller closes a running editor first, through the
// Restart Manager, as its installer did.
procedure UninstallOldPackEditor();
var
  Key: String;
  Uninstaller: String;
  ResultCode: Integer;
begin
  Key := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Lewdware Pack Editor_is1';
  if RegQueryStringValue(HKEY_CURRENT_USER, Key, 'UninstallString', Uninstaller) then
  begin
    Exec(RemoveQuotes(Uninstaller), '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopLewdware();
  UninstallOldPackEditor();
  Result := '';
end;

procedure BroadcastEnvironmentChange();
var
  Dummy: Longint;
begin
  SendMessageTimeoutW(HWND($FFFF), WM_SETTINGCHANGE, 0, 'Environment',
    SMTO_ABORTIFHUNG, 5000, Dummy);
end;

function NeedsAddPath(): Boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath) then
  begin
    Result := True;
    Exit;
  end;
  // Look for the expanded constant {app} in OrigPath (case-insensitive)
  Result := Pos(';' + UpperCase(ExpandConstant('{app}')) + ';', ';' + UpperCase(OrigPath) + ';') = 0;
end;

procedure RemovePath();
var
  Paths: string;
  AppPath: string;
  PosApp: Integer;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Paths) then
    Exit;

  AppPath := ExpandConstant('{app}');
  
  // Loop to remove all instances of the application path
  repeat
    PosApp := Pos(';' + UpperCase(AppPath), UpperCase(Paths));
    if PosApp > 0 then
    begin
      Delete(Paths, PosApp, Length(AppPath) + 1);
    end
    else
    begin
      PosApp := Pos(UpperCase(AppPath) + ';', UpperCase(Paths));
      if PosApp > 0 then
      begin
        Delete(Paths, PosApp, Length(AppPath) + 1);
      end
      else
      begin
        if UpperCase(Paths) = UpperCase(AppPath) then
          Paths := '';
        Break;
      end;
    end;
  until False;

  RegWriteExpandStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Paths);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    BroadcastEnvironmentChange();
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    // Before any file is removed - otherwise the supervisor keeps running against a
    // half-deleted install, and its own .exe can't be deleted at all.
    StopLewdware();
    RemovePath();
  end;
  if CurUninstallStep = usPostUninstall then
    BroadcastEnvironmentChange();
end;
