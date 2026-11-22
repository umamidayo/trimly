@echo off
rem Builds the release exe and the installer into dist\DiscordTrim-Setup.exe.
setlocal
cd /d "%~dp0"

set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
  echo [DiscordTrim] Inno Setup 6 is not installed. Run: winget install JRSoftware.InnoSetup
  exit /b 1
)

echo [DiscordTrim] Building release...
cargo build --release
if errorlevel 1 exit /b 1

echo [DiscordTrim] Building installer...
"%ISCC%" "installer\DiscordTrim.iss"
if errorlevel 1 exit /b 1

echo [DiscordTrim] Done: dist\DiscordTrim-Setup.exe
endlocal
