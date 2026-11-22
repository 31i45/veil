#ifndef AppVersion
  #error "AppVersion must be supplied by tools/installer.ps1"
#endif
#ifndef BundleDir
  #error "BundleDir must be supplied by tools/installer.ps1"
#endif
#ifndef RepoRoot
  #error "RepoRoot must be supplied by tools/installer.ps1"
#endif
#ifndef OutputDirectory
  #error "OutputDirectory must be supplied by tools/installer.ps1"
#endif

[Setup]
AppId={{00420A86-79F1-4485-ABB5-40BB8C2868E9}
AppName=Veil
AppVersion={#AppVersion}
AppVerName=Veil {#AppVersion}
AppPublisher=Veil
VersionInfoCompany=Veil
VersionInfoDescription=Veil Setup
VersionInfoProductName=Veil
VersionInfoVersion={#AppVersion}
VersionInfoTextVersion={#AppVersion}
VersionInfoProductVersion={#AppVersion}
VersionInfoProductTextVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Veil
DefaultGroupName=Veil
PrivilegesRequired=lowest
LicenseFile={#RepoRoot}\LICENSE
OutputDir={#OutputDirectory}
OutputBaseFilename=veil-setup-{#AppVersion}
SetupIconFile={#RepoRoot}\icons\veil.ico
UninstallDisplayIcon={app}\veil.exe
Uninstallable=yes
CreateUninstallRegKey=yes
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#BundleDir}\veil.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\THIRD-PARTY-LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\PRIVACY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\SECURITY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\SUPPORT.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\icons\veil.ico"; DestDir: "{app}\icons"; Flags: ignoreversion

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Icons]
Name: "{group}\Veil"; Filename: "{app}\veil.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Veil"; Filename: "{app}\veil.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[UninstallRun]
Filename: "{app}\veil.exe"; Parameters: "/unregister /quiet"; WorkingDir: "{app}"; RunOnceId: "VeilUnregister"; Flags: skipifdoesntexist
