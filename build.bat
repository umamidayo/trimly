@echo off
rem One-click build: stop DiscordTrim, build release, start the new build.
setlocal
cd /d "%~dp0"

where cargo >nul 2>&1
if errorlevel 1 (
  echo [DiscordTrim] Rust is not installed. Get it from https://rustup.rs and run this again.
  pause
  exit /b 1
)

echo [DiscordTrim] Stopping running instance...
taskkill /f /im DiscordTrim.exe >nul 2>&1
rem Give Windows a moment to release the exe file.
ping -n 2 127.0.0.1 >nul

echo [DiscordTrim] Building release...
cargo build --release
if errorlevel 1 (
  echo.
  echo [DiscordTrim] BUILD FAILED. The new version was not started.
  if exist "target\release\DiscordTrim.exe" (
    echo [DiscordTrim] Restarting the previous build so Alt+Tab keeps working.
    start "" "target\release\DiscordTrim.exe"
  )
  pause
  exit /b 1
)

echo [DiscordTrim] Starting new build...
start "" "target\release\DiscordTrim.exe"
endlocal
