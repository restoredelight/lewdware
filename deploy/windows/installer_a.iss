; installer_a.iss
; Inno Setup Script for Lewdware Main Suite (Installer A)

[Setup]
AppName=Lewdware
AppVersion=0.1.0
DefaultDirName={localappdata}\Programs\Lewdware
DefaultGroupName=Lewdware
OutputDir=..\..\dist
OutputBaseFilename=Lewdware-Installer-x64
Compression=lzma2/max
SolidCompression=yes
ChangesEnvironment=yes
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
SetupIconFile=..\..\config\src-tauri\icons\icon.ico

[Files]
; Main config GUI app
Source: "..\..\target\release\lewdware-config.exe"; DestDir: "{app}"; DestName: "lewdware-config.exe"; Flags: ignoreversion
; Engine
Source: "..\..\target\release\lewdware.exe"; DestDir: "{app}"; Flags: ignoreversion
; CLI
Source: "..\..\target\release\lw.exe"; DestDir: "{app}"; Flags: ignoreversion
; DLLs (copied from staging)
Source: "..\..\target\release\*.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; Visual C++ Redistributable (extracted to temp and deleted after install)
Source: "..\..\build\win-stage\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall

[Icons]
Name: "{group}\Lewdware"; Filename: "{app}\lewdware.exe"
Name: "{group}\Lewdware Config"; Filename: "{app}\lewdware-config.exe"
Name: "{userdesktop}\Lewdware"; Filename: "{app}\lewdware.exe"; Tasks: desktopicon
Name: "{userdesktop}\Lewdware Config"; Filename: "{app}\lewdware-config.exe"; Tasks: desktopicon

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
    RemovePath();
  end;
  if CurUninstallStep = usPostUninstall then
    BroadcastEnvironmentChange();
end;
