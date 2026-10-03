#ifndef AppVersion
  #error AppVersion must be supplied by build-installer.ps1
#endif
#ifndef PayloadDir
  #error PayloadDir must be supplied by build-installer.ps1
#endif
#ifndef OutputDir
  #error OutputDir must be supplied by build-installer.ps1
#endif
#ifndef InstallerAppId
  #define InstallerAppId "QuotaScope.Windows"
#endif
#ifndef OutputName
  #define OutputName "QuotaScope-" + AppVersion + "-windows-x64-Setup"
#endif
#ifndef InstallerGroupName
  #define InstallerGroupName "QuotaScope"
#endif
#ifndef InstallerDefaultDir
  #define InstallerDefaultDir "{localappdata}\Programs\QuotaScope"
#endif

[Setup]
AppId={#InstallerAppId}
AppName=QuotaScope
AppVersion={#AppVersion}
AppPublisher=Lyx721188
AppPublisherURL=https://github.com/Lyx721188/QuotaScope
DefaultDirName={#InstallerDefaultDir}
DefaultGroupName={#InstallerGroupName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
SetupIconFile=..\quotascope-win\assets\app.ico
UninstallDisplayIcon={app}\quotascope.exe
LicenseFile=..\..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
DisableProgramGroupPage=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\QuotaScope"; Filename: "{app}\quotascope.exe"; WorkingDir: "{app}"
Name: "{group}\QuotaScope Settings"; Filename: "{app}\quotascope.exe"; Parameters: "--settings"; WorkingDir: "{app}"
Name: "{autodesktop}\QuotaScope"; Filename: "{app}\quotascope.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\quotascope.exe"; Parameters: "--settings"; Description: "Launch QuotaScope"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  StartupCommand: String;
begin
  if CurUninstallStep = usUninstall then
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'QuotaScope', StartupCommand) then
      if CompareText(StartupCommand, '"' + ExpandConstant('{app}\quotascope.exe') + '"') = 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'QuotaScope');
end;
