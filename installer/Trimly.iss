; Inno Setup 6.7+ script for Trimly. Build with release.bat (or ISCC.exe on this file)
; after `cargo build --release`. Keep AppVersion equal to the version in Cargo.toml
; (build.rs checks). Artwork comes from res\make-art.ps1.

#define AppVersion "0.1.0"
#ifndef ExeDir
#define ExeDir "..\target\release"
#endif

[Setup]
; Fixed AppId so each new version upgrades the old install in place.
AppId={{BCCD887A-BBFF-4866-A6A7-D66B121084B9}
AppName=Trimly
AppVersion={#AppVersion}
AppVerName=Trimly {#AppVersion}
AppPublisher=Trimly
AppPublisherURL=https://github.com/umamidayo/trimly
AppSupportURL=https://github.com/umamidayo/trimly/issues
AppUpdatesURL=https://github.com/umamidayo/trimly/releases
AppCopyright=Copyright (c) 2026 Trimly contributors
VersionInfoDescription=Trimly Setup
VersionInfoProductName=Trimly
DefaultDirName={autopf}\Trimly
DefaultGroupName=Trimly
DisableProgramGroupPage=yes
DisableReadyPage=yes
DisableWelcomePage=no
UninstallDisplayIcon={app}\Trimly.exe
UninstallDisplayName=Trimly
SetupIconFile=..\res\app.ico
OutputDir=..\dist
OutputBaseFilename=Trimly-Setup
Compression=lzma2/max
SolidCompression=yes
; Installs per user by default, so no administrator prompt.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
; Closes the running tray app before replacing it, and asks the user first.
CloseApplications=yes
RestartApplications=no
; The look: Windows 11 styling that follows the user's light/dark setting, in Trimly's colors,
; with Trimly's own banner and logo. Several image sizes keep it sharp at every display scale.
WizardStyle=modern dynamic windows11 hidebevels
WizardBackColor=#f4f6f8
WizardBackColorDynamicDark=#0c1115
; Dark mode swaps in Inno's own dark images unless dark ones are named too. The artwork is
; dark already, so both appearances use the same files.
WizardImageFile=..\res\installer\wizard-100.png,..\res\installer\wizard-150.png,..\res\installer\wizard-200.png
WizardImageFileDynamicDark=..\res\installer\wizard-100.png,..\res\installer\wizard-150.png,..\res\installer\wizard-200.png
WizardSmallImageFile=..\res\installer\small-100.png,..\res\installer\small-150.png,..\res\installer\small-200.png
WizardSmallImageFileDynamicDark=..\res\installer\small-100.png,..\res\installer\small-150.png,..\res\installer\small-200.png
; The wizard is drawn 120% larger than its 96-DPI layout; scale the banner to fill its panel.
WizardImageStretch=yes

[Messages]
WelcomeLabel1=Welcome to Trimly
WelcomeLabel2=Trimly keeps the apps you choose lean on memory, so the ones you're using get the RAM.%n%nIt runs quietly in your tray, never changes the apps themselves, and you decide which apps it touches.
WizardSelectTasks=Options
SelectTasksDesc=Pick how you'd like to use Trimly.
SelectTasksLabel2=You can change these later in Trimly's Settings.
FinishedHeadingLabel=Trimly is ready
FinishedLabelNoIcons=Trimly is installed. Open it, add the apps you want kept lean, and it takes care of the rest.
FinishedLabel=Trimly is installed. Open it, add the apps you want kept lean, and it takes care of the rest.

[Files]
Source: "{#ExeDir}\Trimly.exe"; DestDir: "{app}"; Flags: ignoreversion

[Tasks]
Name: "autostart"; Description: "Start Trimly when I sign in to Windows"
Name: "desktopicon"; Description: "Add a desktop shortcut"

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
Filename: "{app}\Trimly.exe"; Description: "Open Trimly now"; Flags: nowait postinstall skipifsilent
