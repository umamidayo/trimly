@echo off
rem Builds the release exe and the installer into dist\Trimly-Setup.exe.
setlocal
cd /d "%~dp0"

set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
  echo [Trimly] Inno Setup 6 is not installed. Run: winget install JRSoftware.InnoSetup
  exit /b 1
)

echo [Trimly] Building release...
rem Replace this machine's folders (user name, project path) in the exe's built-in source paths,
rem so a published build doesn't reveal who built it or where.
cargo build --release --config "build.rustflags=['--remap-path-prefix=%USERPROFILE%=~', '--remap-path-prefix=%CD%=trimly']"
if errorlevel 1 exit /b 1

echo [Trimly] Building installer...
"%ISCC%" "installer\Trimly.iss"
if errorlevel 1 exit /b 1

echo [Trimly] Done: dist\Trimly-Setup.exe
endlocal
