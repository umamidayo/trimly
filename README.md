# Trimly

A small Windows tray app that keeps the apps you choose lean on memory: Discord, Steam, your browser, anything. It never modifies those apps or their files, so their updates don't affect it, and every feature keeps working. Written in Rust with direct Win32 calls; the window is an HTML page in WebView2.

## Install

Download `Trimly-Setup.exe` from the [latest release](https://github.com/umamidayo/trimly/releases/latest) and run it. It installs for your user only, with no administrator prompt, and offers two options: start with Windows, and a desktop shortcut.

## How it works

Add apps in the **Apps** tab. The picker lists running programs by name and memory use. An app's helper processes are included automatically: Steam brings `steamwebhelper.exe`, because both are installed in the same folder.

Each app has one choice of **when to trim**:

| Mode | What happens |
| --- | --- |
| When hidden | Trimmed once none of its windows have been open for one interval, then every interval while it stays hidden. |
| Always | Trimmed every interval, even while you use it. For apps kept open on a second monitor. May cause a brief pause. |
| Paused | Not trimmed. Its memory is still shown. |

and one switch, **efficiency mode while hidden**: Windows' EcoQoS, which lowers the app's CPU priority while none of its windows are open and is removed the moment one opens.

Trimming calls `EmptyWorkingSet`: memory the app isn't using moves out of RAM, and Windows loads it back on demand. The app keeps its memory and behaves the same. "Shown" means a visible, restored, titled window; tool windows such as Discord's in-game overlay don't count.

The **Settings** tab has the two intervals (hidden apps, default 1 min; always-on apps, default 5 min), **Pause trimming**, **Start with Windows**, and the theme.

## The window

- **Overview**: total RAM of your apps with a 10-minute chart, **Trim all now**, system memory, and each app with its own trim button.
- **Apps**: add, configure and remove apps.
- **Stats**: memory freed all time and this session, biggest single trim, average per trim, a per-app breakdown, and recent activity. All-time numbers are kept in `stats.json`.
- **Settings**: trimming, appearance, general.

Themes: **Mint** (default dark), **Ember** (warm dark), **Daylight** (light), or **Match Windows**. The window is resizable, remembers its size, and uses two columns when wide. Its WebView2 is shut down when you close it, so it costs no memory in the background. Left-click the tray icon to open it; right-click for the menu.

On start, Trimly checks this repo's latest release over HTTPS. If it's newer, the window shows **Download**, which opens the installer's link in your browser. Trimly never downloads or runs anything itself.

## Data

Everything lives in `%APPDATA%\Trimly`: `config.json` (apps and settings), `stats.json` (all-time totals) and `WebView2\` (the window's browser cache). Older configs from before the app list existed are converted automatically.

## Antivirus false positives

Trimly opens other processes and changes their memory and CPU priority, which is also what some malware does. Small unsigned programs like this one can trip Windows Defender's machine-learning detections (for example `Trojan:Win32/Bearfoos.A!ml`). To keep that unlikely, Trimly:

- never downloads or launches executables (updates go through the browser),
- never adds itself to startup on its own (only when you tick it),
- starts no hidden child processes,
- carries full version information in the exe.

The remaining step is code signing; see [Releasing](#releasing-a-new-version). If a build is flagged anyway, report it to Microsoft as a false positive at <https://www.microsoft.com/wdsi/filesubmission>.

## Build and run

1. Install Rust once: https://rustup.rs (pick the default MSVC toolchain).
2. Double-click **`build.bat`**. It closes any running Trimly, builds the release exe, and starts the new one.

`cargo test` runs the unit tests.

## Releasing a new version

1. Bump the version in `Cargo.toml`, `res/app.rc` (four places) and `installer/Trimly.iss`. The build stops if they disagree.
2. Run `release.bat`. It builds `dist\Trimly-Setup.exe` (needs [Inno Setup 6](https://jrsoftware.org/isinfo.php)).
3. Sign `target\release\Trimly.exe` and the installer before publishing, if you have a certificate.
4. Publish a GitHub release tagged `vX.Y.Z` with `Trimly-Setup.exe` attached.

## Project layout

| File | Purpose |
| --- | --- |
| `src/main.rs` | Single-instance check, starts the tray |
| `src/tray.rs` | Tray icon, owner-drawn menu, message loop |
| `src/engine.rs` | Background thread: measures, trims, efficiency mode, statistics |
| `src/settings.rs` | The window: WebView2 host, custom title bar and resizing, page commands, app picker |
| `src/ui.html` | The page (Overview, Apps, Stats, Settings) |
| `src/config.rs` | Load/save `config.json`, migration from old configs |
| `src/procs.rs` | Process list, paths, names, memory, window visibility |
| `src/theme.rs` | Theme palettes for the native parts |
| `src/update.rs` | GitHub Releases check (WinHTTP) |
| `src/autostart.rs` | Start with Windows (HKCU Run key) |
| `res/` | Icon, version info, manifest |
| `installer/Trimly.iss` | Inno Setup script |

## Known limits

- A program's processes are grouped by install folder. If an app's helpers live somewhere else, add them as their own app.
- Trimming saves the most on apps that sit in the background for a while. Right after a trim, Windows pages back in what's needed, which can cause a short pause.
- Some processes (e.g. elevated ones) can only be trimmed when Trimly itself runs as administrator (tray menu > Restart as administrator).
