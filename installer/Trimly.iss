; Inno Setup script for Trimly. Build with release.bat (or ISCC.exe on this file)
; after `cargo build --release`. Keep AppVersion equal to the version in Cargo.toml.

#define AppVersion "0.1.0"
#ifndef ExeDir
#define ExeDir "..\target\release"
#endif

[Setup]
; Fixed AppId so each new version upgrades the old install in place.
AppId={{BCCD887A-BBFF-4866-A6A7-D66B121084B9}
AppName=Trimly
AppVersion={#AppVersion}
AppPublisher=Trimly
DefaultDirName={autopf}\Trimly
DefaultGroupName=Trimly
UninstallDisplayIcon={app}\Trimly.exe
SetupIconFile=..\res\app.ico
OutputDir=..\dist
OutputBaseFilename=Trimly-Setup
Compression=lzma2
SolidCompression=yes
; Installs per user by default, so no administrator prompt. The app's "Restart as administrator"
; menu item still works for the elevated trim of Discord's main process.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; Closes the running tray app before replacing it, and asks the user first.
CloseApplications=yes
RestartApplications=no

[Files]
Source: "{#ExeDir}\Trimly.exe"; DestDir: "{app}"; Flags: ignoreversion

[Tasks]
Name: "autostart"; Description: "Start Trimly when I sign in to Windows"
Name: "desktopicon"; Description: "Create a desktop shortcut"

[Icons]
Name: "{group}\Trimly"; Filename: "{app}\Trimly.exe"
Name: "{group}\Uninstall Trimly"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Trimly"; Filename: "{app}\Trimly.exe"; Tasks: desktopicon

[Registry]
; Start with Windows, only when the user ticks it here (the app never adds itself on its own).
; uninsdeletevalue also removes it if it was turned on later from inside the app.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Trimly"; ValueData: """{app}\Trimly.exe"""; Flags: uninsdeletevalue; Tasks: autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "Trimly"; Flags: uninsdeletevalue dontcreatekey; Check: not WizardIsTaskSelected('autostart')

[Run]
Filename: "{app}\Trimly.exe"; Description: "Start Trimly"; Flags: nowait postinstall skipifsilent
