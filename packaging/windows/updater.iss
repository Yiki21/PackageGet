#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef BinaryPath
  #error BinaryPath is required
#endif
#ifndef NumericVersion
  #error NumericVersion is required
#endif
#ifndef IconPath
  #error IconPath is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId=com.ayi.updater
AppName=Updater
AppVersion={#AppVersion}
AppPublisher=Yiki21
AppPublisherURL=https://github.com/Yiki21/PackageGet
AppSupportURL=https://github.com/Yiki21/PackageGet/issues
DefaultDirName={autopf}\Updater
DefaultGroupName=Updater
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=updater-{#AppVersion}-windows-x86_64-setup
SetupIconFile={#IconPath}
UninstallDisplayIcon={app}\updater.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
VersionInfoVersion={#NumericVersion}
VersionInfoCompany=Yiki21
VersionInfoDescription=Updater installer
VersionInfoProductName=Updater
VersionInfoProductVersion={#NumericVersion}

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Updater"; Filename: "{app}\updater.exe"
Name: "{userdesktop}\Updater"; Filename: "{app}\updater.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Run]
Filename: "{app}\updater.exe"; Description: "Launch Updater"; Flags: nowait postinstall skipifsilent
