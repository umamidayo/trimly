//! Trims an app's resident memory once its windows have been hidden or minimized for a while.
//! Calls EmptyWorkingSet, so pages that are not in use move out of RAM. The app keeps its
//! memory and behavior; pages come back on demand when the window is restored.

use super::Tweak;
use crate::config::Config;
use crate::procs;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::ProcessStatus::{EmptyWorkingSet, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};

/// Running totals across all trims, shown in the settings window.
pub static TRIMS: AtomicU64 = AtomicU64::new(0);
/// Bytes that left the working set during trims (measured before and after each EmptyWorkingSet).
pub static FREED_BYTES: AtomicU64 = AtomicU64::new(0);
/// The same freed bytes, split by app name.
pub static FREED_BY_APP: Mutex<BTreeMap<&'static str, u64>> = Mutex::new(BTreeMap::new());

/// An app Trimly watches: its display name, every exe it runs as, and the features that trim it.
pub struct Tracked {
    pub name: &'static str,
    pub exes: &'static [&'static str],
    pub features: &'static [&'static str],
}

pub const DISCORD_EXES: &[&str] = &["discord.exe"];
pub const EXPLORER_EXES: &[&str] = &["explorer.exe"];

/// Every app shown in the Overview, in display order.
pub const APPS: [Tracked; 4] = [
    Tracked { name: "Discord", exes: DISCORD_EXES, features: &["discord_trim_idle", "discord_trim_active"] },
    Tracked { name: "Steam", exes: &["steam.exe", "steamwebhelper.exe"], features: &["steam_trim_idle"] },
    Tracked { name: "Windows Explorer", exes: EXPLORER_EXES, features: &["explorer_trim_idle"] },
    Tracked { name: "CTF Loader", exes: &["ctfmon.exe"], features: &["ctf_trim_idle"] },
];

/// One trim feature: its config key, tray label, the app it belongs to and that app's exes.
#[derive(Clone, Copy)]
pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub app: &'static str,
    pub exes: &'static [&'static str],
}

pub const DISCORD: Target = Target {
    id: "discord_trim_idle",
    name: "Trim Discord memory when hidden",
    app: "Discord",
    exes: DISCORD_EXES,
};

/// CTF Loader (text services). It has no visible windows, so it is always idle.
pub const CTF_LOADER: Target = Target {
    id: "ctf_trim_idle",
    name: "Trim CTF Loader memory when hidden",
    app: "CTF Loader",
    exes: &["ctfmon.exe"],
};

/// Steam's client plus its Chromium web helper, which holds most of Steam's memory.
pub const STEAM: Target = Target {
    id: "steam_trim_idle",
    name: "Trim Steam memory when hidden",
    app: "Steam",
    exes: &["steam.exe", "steamwebhelper.exe"],
};

/// Discord trimmed on a timer while its windows are shown too, for Discord kept open on another
/// monitor. A trim on visible Discord can cause a short pause when it pages memory back in.
pub const DISCORD_ACTIVE: Target = Target {
    id: "discord_trim_active",
    name: "Trim Discord memory while visible",
    app: "Discord",
    exes: DISCORD_EXES,
};

/// Timing settings shared with the polling thread, so changes apply without a restart.
struct Timing {
    delay_secs: AtomicU64,
    interval_secs: AtomicU64,
    active_interval_secs: AtomicU64,
}

pub struct TrimIdle {
    target: Target,
    /// Trim on a timer whatever the window state, instead of only after the app is hidden.
    active: bool,
    stop: Option<Arc<AtomicBool>>,
    timing: Arc<Timing>,
    thread: Option<JoinHandle<()>>,
}

impl TrimIdle {
    pub fn new(target: Target) -> Self {
        Self::with_mode(target, false)
    }

    pub fn new_active(target: Target) -> Self {
        Self::with_mode(target, true)
    }

    fn with_mode(target: Target, active: bool) -> Self {
        let timing = Timing {
            delay_secs: AtomicU64::new(30),
            interval_secs: AtomicU64::new(60),
            active_interval_secs: AtomicU64::new(300),
        };
        Self { target, active, stop: None, timing: Arc::new(timing), thread: None }
    }

    fn load_timing(&self, cfg: &Config) {
        self.timing.delay_secs.store(cfg.trim_delay_secs, Ordering::Relaxed);
        self.timing.interval_secs.store(cfg.trim_interval_secs, Ordering::Relaxed);
        self.timing.active_interval_secs.store(cfg.active_trim_interval_secs, Ordering::Relaxed);
    }
}

impl Tweak for TrimIdle {
    fn id(&self) -> &'static str {
        self.target.id
    }
    fn name(&self) -> &'static str {
        self.target.name
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn enable(&mut self, cfg: &Config) -> Result<(), String> {
        self.load_timing(cfg);
        let stop = Arc::new(AtomicBool::new(false));
        let (s, t, target, active) = (stop.clone(), self.timing.clone(), self.target, self.active);
        self.thread = Some(thread::spawn(move || run(&s, &t, target, active)));
        self.stop = Some(stop);
        Ok(())
    }
    fn disable(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        // The loop exits within one poll tick; no need to block the menu thread on join.
        self.thread.take();
    }
    fn is_enabled(&self) -> bool {
        self.stop.is_some()
    }
    fn apply_config(&mut self, cfg: &Config) {
        self.load_timing(cfg);
    }
}

/// Polls the app every couple of seconds. Once every window has been hidden or minimized for
/// `delay_secs`, trims each of its processes. While it stays hidden, each process is trimmed
/// again every `interval_secs`, because memory climbs back after a trim. Any shown window
/// resets the cycle.
/// Active mode ignores window state and trims every `active_interval_secs`.
fn run(stop: &AtomicBool, timing: &Timing, target: Target, active: bool) {
    let mut idle_since: Option<Instant> = None;
    let mut last_trim: HashMap<u32, Instant> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let pids = procs::pids(target.exes);
        if active {
            let interval = Duration::from_secs(timing.active_interval_secs.load(Ordering::Relaxed));
            trim_due(&pids, &mut last_trim, interval, target.app);
        } else if procs::any_window_shown(&pids) {
            idle_since = None;
            last_trim.clear();
        } else {
            let since = *idle_since.get_or_insert_with(Instant::now);
            let delay = Duration::from_secs(timing.delay_secs.load(Ordering::Relaxed));
            let interval = Duration::from_secs(timing.interval_secs.load(Ordering::Relaxed));
            if since.elapsed() >= delay {
                trim_due(&pids, &mut last_trim, interval, target.app);
            }
        }
        sleep_unless_stopped(stop, POLL);
    }
}

/// Trims each pid whose last trim is at least `interval` old.
fn trim_due(pids: &[u32], last_trim: &mut HashMap<u32, Instant>, interval: Duration, app: &'static str) {
    for &pid in pids {
        let due = last_trim.get(&pid).map_or(true, |t| t.elapsed() >= interval);
        if due {
            trim(pid, app);
            last_trim.insert(pid, Instant::now());
        }
    }
}

const POLL: Duration = Duration::from_secs(2);

/// Trims one process of `app`, records how many bytes left its working set, and returns that.
pub fn trim(pid: u32, app: &'static str) -> u64 {
    let Some(h) = procs::open(pid) else { return 0 };
    let before = working_set(h);
    unsafe { EmptyWorkingSet(h) };
    let after = working_set(h);
    procs::close(h);
    let freed = before.saturating_sub(after);
    TRIMS.fetch_add(1, Ordering::Relaxed);
    FREED_BYTES.fetch_add(freed, Ordering::Relaxed);
    if let Ok(mut by_app) = FREED_BY_APP.lock() {
        *by_app.entry(app).or_default() += freed;
    }
    freed
}

/// Trims every process of a tracked app right now ("Trim" in the Overview). Returns bytes freed.
pub fn trim_app(app: &Tracked) -> u64 {
    procs::pids(app.exes).into_iter().map(|pid| trim(pid, app.name)).sum()
}

/// Current RAM (working set) of a tracked app across its processes, or None when it isn't running.
/// Uses query-only handles, so elevated processes count too.
pub fn app_ram(app: &Tracked) -> Option<u64> {
    let pids = procs::pids(app.exes);
    if pids.is_empty() {
        return None;
    }
    Some(pids.into_iter().filter_map(procs::open_query).map(|h| {
        let bytes = working_set(h);
        procs::close(h);
        bytes
    }).sum())
}

pub fn working_set(h: windows_sys::Win32::Foundation::HANDLE) -> u64 {
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { GetProcessMemoryInfo(h, &mut c, c.cb) };
    if ok == 0 { 0 } else { c.WorkingSetSize as u64 }
}

/// Sleeps for `total`, but wakes early in short steps so disable() takes effect quickly.
fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(200);
    let mut slept = Duration::ZERO;
    while slept < total && !stop.load(Ordering::Relaxed) {
        thread::sleep(step);
        slept += step;
    }
}
