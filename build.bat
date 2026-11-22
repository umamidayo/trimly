@echo off
rem One-click build: stop Trimly, build release, start the new build.
setlocal
cd /d "%~dp0"

where cargo >nul 2>&1
if errorlevel 1 (
  echo [Trimly] Rust is not installed. Get it from https://rustup.rs and run this again.
  pause
  exit /b 1
)

echo [Trimly] Stopping running instance...
taskkill /f /im Trimly.exe >nul 2>&1
rem Give Windows a moment to release the exe file.
ping -n 2 127.0.0.1 >nul

echo [Trimly] Building release...
rem Same path remapping as release.bat, so both scripts share one build cache.
cargo build --release --config "build.rustflags=['--remap-path-prefix=%USERPROFILE%=~', '--remap-path-prefix=%CD%=trimly']"
if errorlevel 1 (
  echo.
  echo [Trimly] BUILD FAILED. The new version was not started.
  if exist "target\release\Trimly.exe" (
    echo [Trimly] Restarting the previous build so Alt+Tab keeps working.
    start "" "target\release\Trimly.exe"
  )
  pause
  exit /b 1
)

echo [Trimly] Starting new build...
start "" "target\release\Trimly.exe"
endlocal
