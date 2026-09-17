#ifndef MyAppVersion
  #define MyAppVersion "1.1.0"
#endif

#define MyAppName "VulpiCast"
#define MyAppPublisher "TecTolik"
#define MyAppURL "https://github.com/TecTolik/VulpiCast"
#define ProjectRoot SourcePath + "..\"

[Setup]
AppId={{F8B5B53C-3E82-46ED-A35C-1AF406825250}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile={#ProjectRoot}LICENSE
OutputDir={#ProjectRoot}dist
OutputBaseFilename=VulpiCast-Setup
SetupIconFile={#ProjectRoot}assets\vulpicast.ico
UninstallDisplayIcon={app}\VulpiCast.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=yes
RestartApplications=no
VersionInfoVersion={#MyAppVersion}.0
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription=VulpiCast Installer

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#ProjectRoot}target\release\vulpicast.exe"; DestDir: "{app}"; DestName: "VulpiCast.exe"; Flags: ignoreversion
Source: "{#ProjectRoot}LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\VulpiCast"; Filename: "{app}\VulpiCast.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\VulpiCast"; Filename: "{app}\VulpiCast.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""VulpiCast (AirPlay 2)"""; Flags: runhidden waituntilterminated
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""VulpiCast (AirPlay 2)"" dir=in action=allow program=""{app}\VulpiCast.exe"" enable=yes profile=private"; Flags: runhidden waituntilterminated
Filename: "{app}\VulpiCast.exe"; Description: "Launch VulpiCast now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""VulpiCast (AirPlay 2)"""; Flags: runhidden waituntilterminated; RunOnceId: "RemoveVulpiCastFirewallRule"
