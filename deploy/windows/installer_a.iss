; installer_a.iss
; Inno Setup Script for Lewdware Main Suite (Installer A)

[Setup]
AppName=Lewdware Main Suite
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
SetupIconFile=..\..\config-tauri\src-tauri\icons\icon.ico

[Files]
; Main config GUI app
Source: "..\..\target\release\config-tauri.exe"; DestDir: "{app}"; DestName: "config.exe"; Flags: ignoreversion
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
Name: "{group}\Lewdware Configurator"; Filename: "{app}\config.exe"
Name: "{userdesktop}\Lewdware"; Filename: "{app}\lewdware.exe"; Tasks: desktopicon
Name: "{userdesktop}\Lewdware Configurator"; Filename: "{app}\config.exe"; Tasks: desktopicon

[Run]
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing Visual C++ Redistributable..."

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "addtopath"; Description: "Add 'lw' CLI to the user PATH environment variable"; GroupDescription: "Environment Setup:"

[Registry]
; Update the User PATH variable to expose 'lw' CLI
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Flags: preservestringtype; Tasks: addtopath

[Code]
procedure BroadcastEnvironmentChange();
var
  Temp: Longint;
begin
  SendMessageTimeout(HWND_BROADCAST, WM_SETTINGCHANGE, 0,
    CastStringToInteger('Environment'), SMTO_ABORTIFHUNG, 5000, Temp);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    BroadcastEnvironmentChange();
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    BroadcastEnvironmentChange();
end;
