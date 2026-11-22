; Inno Setup script for DiscordTrim. Build with release.bat (or ISCC.exe on this file)
; after `cargo build --release`. Keep AppVersion equal to the version in Cargo.toml.

#define AppVersion "0.1.0"

[Setup]
; Fixed AppId so each new version upgrades the old install in place.
AppId={{BCCD887A-BBFF-4866-A6A7-D66B121084B9}
AppName=DiscordTrim
AppVersion={#AppVersion}
AppPublisher=DiscordTrim
DefaultDirName={autopf}\DiscordTrim
DefaultGroupName=DiscordTrim
UninstallDisplayIcon={app}\DiscordTrim.exe
SetupIconFile=..\res\app.ico
OutputDir=..\dist
OutputBaseFilename=DiscordTrim-Setup
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
Source: "..\target\release\DiscordTrim.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\DiscordTrim"; Filename: "{app}\DiscordTrim.exe"
Name: "{group}\Uninstall DiscordTrim"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\DiscordTrim.exe"; Description: "Start DiscordTrim"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Remove the "Start with Windows" entry the app adds, so Windows doesn't launch a deleted exe.
Filename: "{sys}\reg.exe"; Parameters: "delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v DiscordTrim /f"; Flags: runhidden; RunOnceId: "RemoveAutostart"
