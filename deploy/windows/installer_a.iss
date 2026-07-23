; installer_a.iss
; Inno Setup Script for Lewdware Main Suite (Installer A)

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
SetupIconFile=..\..\config\src-tauri\icons\icon.ico
; Without this the Apps & features entry has no icon next to it.
UninstallDisplayIcon={app}\lewdware.exe
UninstallDisplayName=Lewdware
; The bundled DLLs and the vc_redist are x64-only.
ArchitecturesAllowed=x64compatible
LicenseFile=..\..\LICENSE
; StopLewdware (see [Code]) shuts everything down in PrepareToInstall, which runs *before* this
; check - so it normally finds nothing. Left enabled as a backstop for anything not covered
; there, e.g. a stray lw.exe running out of {app}.
CloseApplications=yes
; ...but don't let the Restart Manager relaunch the supervisor behind the user's back once the
; install finishes. Starting a session is the user's call.
RestartApplications=no

[Files]
; MIT license - shown during install above, and kept alongside the binaries
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
; Config GUI (user-facing entry point)
Source: "..\..\target\release\lewdware.exe"; DestDir: "{app}"; DestName: "lewdware.exe"; Flags: ignoreversion
; Engine (internal, launched by config app)
Source: "..\..\target\release\lewdware-engine.exe"; DestDir: "{app}"; Flags: ignoreversion
; Supervisor (internal, owns session lifecycle and scheduling)
Source: "..\..\target\release\lewdware-supervisor.exe"; DestDir: "{app}"; Flags: ignoreversion
; CLI
Source: "..\..\target\release\lw.exe"; DestDir: "{app}"; Flags: ignoreversion
; DLLs (copied from staging)
Source: "..\..\target\release\*.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; Visual C++ Redistributable (extracted to temp and deleted after install)
Source: "..\..\build\win-stage\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{group}\Lewdware"; Filename: "{app}\lewdware.exe"
Name: "{userdesktop}\Lewdware"; Filename: "{app}\lewdware.exe"; Tasks: desktopicon

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

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopLewdware();
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
