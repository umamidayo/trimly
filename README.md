# DiscordTrim

A tiny Windows tray app that lowers Discord's memory use while Discord is in the background. It does not modify Discord's files, so Discord updates don't affect it, and every Discord feature keeps working. Written in Rust with direct Win32 calls, the same way as DisplayNova.

## What it does

**Trim Discord memory when hidden.** When none of Discord's windows are shown for `trim_delay_secs` (default 300 seconds), it calls `EmptyWorkingSet` on each Discord process. Memory Discord isn't using moves out of RAM, and Windows loads it back on demand. Task Manager's memory column drops right away. The saving is real but depends on how much Discord had in use. In a test on an already-hidden Discord it freed about 16 MB out of 1040 MB.

**Efficiency mode for hidden Discord.** While no Discord window is shown, its processes are put into Windows efficiency mode (EcoQoS). This lowers CPU scheduling and power use. It doesn't change what Discord does. It's removed the moment a window is restored.

"Shown" means a visible, restored, titled Discord window. Tool windows such as the in-game overlay don't count, so the overlay doesn't keep Discord out of the idle state.

Both features poll every 2 seconds and stop as soon as you turn them off. Both are on by default. Toggle either one from the tray menu.

**Trim Discord memory while visible (on by default).** Trims Discord every `active_trim_interval_secs` (default 300) even when its windows are shown, for when Discord is on another monitor. A trim on visible Discord can cause a short pause while it pages memory back in.

**Settings window.** Left-click the tray icon, or launch DiscordTrim again (for example from the Start menu), to open it. Right-click the tray icon for the menu. It shows measured results at the top: Discord's RAM right now, and the trims run and memory freed across all apps. "Freed" is the working-set drop measured around each `EmptyWorkingSet` call. Below that, every feature has a checkbox and every timing has a number box.

**Updates.** On start, the app checks this repo's latest GitHub release. If it's newer, the Settings window shows **Install update vX**, which downloads and runs the installer. Nothing installs without that click.

## Releasing a new version

1. Bump the version in `Cargo.toml` and in `installer/DiscordTrim.iss`.
2. Run `release.bat`. It builds `dist\DiscordTrim-Setup.exe` (needs [Inno Setup 6](https://jrsoftware.org/isinfo.php)).
3. Publish a GitHub release tagged `vX.Y.Z` with `DiscordTrim-Setup.exe` attached.

## What it does not do

- It doesn't cap Discord's heap or set memory limits. A limit can crash Discord, so none is applied.
- It doesn't touch Discord's settings, files, or network traffic.
- It doesn't change push-to-talk. PTT is handled by Discord's own global hook, which this app doesn't touch.

## Install

Download `DiscordTrim-Setup.exe` from the [latest release](https://github.com/umamidayo/discord-trim/releases/latest) and run it. It installs for your user only, with no administrator prompt. Windows SmartScreen may warn because the installer isn't code-signed.

## Build and run

1. Install Rust once: https://rustup.rs (pick the default MSVC toolchain).
2. Double-click **`build.bat`**. It closes any running DiscordTrim, builds the release exe, and starts the new one.

The exe is `target\release\DiscordTrim.exe`. On first launch it turns on Start with Windows (untick it in the tray menu). Settings live in `%APPDATA%\DiscordTrim\config.json`:

```json
{
  "tweaks": {},
  "trim_delay_secs": 30,
  "trim_interval_secs": 60,
  "active_trim_interval_secs": 300,
  "explorer_idle_secs": 10,
  "explorer_interval_secs": 60,
  "first_run_done": true
}
```

## Adding a feature

1. Create `src/tweaks/my_feature.rs` with a struct that implements the `Tweak` trait in `src/tweaks/mod.rs`.
2. Add `pub mod my_feature;` and push it into `all()` in `src/tweaks/mod.rs`.

It then appears in the tray menu and its on/off state is saved to config.json.

## Project layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Single-instance check, starts the tray |
| `src/tray.rs` | Tray icon, menu, message loop |
| `src/config.rs` | Load/save config.json |
| `src/autostart.rs` | Start with Windows (HKCU Run key) |
| `src/settings.rs` | Settings window and measured results |
| `src/procs.rs` | Finds app processes and checks for shown windows |
| `src/tweaks/mod.rs` | `Tweak` trait and registry |
| `src/tweaks/trim.rs` | Working-set trim (hidden and visible modes) and the measured totals |
| `src/tweaks/throttle.rs` | EcoQoS for hidden Discord processes |
| `res/` | Icon and manifest (per-monitor DPI aware) |

## Known limits

- Discord is detected by process name (`Discord.exe`). Discord PTB and Canary use different names and aren't covered.
- Trimming saves the most when Discord has been idle a long time. Right after you restore its window, Windows pages in what's needed, which can cause a short pause.
- Efficiency mode affects CPU scheduling only, so it saves less power or CPU than RAM.
