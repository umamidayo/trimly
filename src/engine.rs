//! The trimming engine. One background thread visits every app the user added, every 2 seconds:
//!   - measures its RAM (working set) across all its processes,
//!   - trims it when due: "Hidden" apps once none of their windows have been shown for one
//!     hidden-interval and then every interval; "Always" apps every always-interval,
//!   - puts it in Windows efficiency mode (EcoQoS) while hidden, if the app has that on.
//! It also keeps the statistics shown in the window, and saves the all-time ones to stats.json.
//!
//! Trimming calls EmptyWorkingSet: pages the app isn't using move out of RAM and come back on
//! demand. The app keeps its memory and behavior.

use crate::config::{self, AppCfg, Config, Mode};
use crate::procs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{ProcessPowerThrottling, SetProcessInformation, PROCESS_POWER_THROTTLING_STATE};

const POLL: Duration = Duration::from_secs(2);
/// Samples of total app RAM kept for the chart: 10 minutes at one per poll.
const HISTORY: usize = 300;
const EVENTS: usize = 40;

/// Totals that survive restarts (stats.json).
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct AllTime {
    pub freed: u64,
    pub trims: u64,
    /// Unix time the first stats were recorded.
    pub since: u64,
    /// Bytes freed per app name.
    pub per_app: BTreeMap<String, u64>,
    pub biggest: u64,
    pub biggest_app: String,
}

/// Live numbers for one app, this session.
#[derive(Default, Clone)]
pub struct AppStats {
    pub running: bool,
    pub ram: u64,
    pub peak: u64,
    pub procs: usize,
    pub freed: u64,
    pub trims: u64,
    /// Unix time of the last trim.
    pub last_trim: Option<u64>,
}

#[derive(Clone)]
pub struct Event {
    pub at: u64,
    pub app: String,
    pub freed: u64,
}

pub struct Stats {
    pub started: u64,
    pub freed: u64,
    pub trims: u64,
    pub all: AllTime,
    /// By app key (its first exe name).
    pub apps: HashMap<String, AppStats>,
    /// Newest first.
    pub events: VecDeque<Event>,
    /// Total RAM of all added apps, oldest first, one sample per poll.
    pub history: VecDeque<u64>,
}

pub static STATS: LazyLock<Mutex<Stats>> = LazyLock::new(|| {
    let mut all: AllTime = std::fs::read_to_string(stats_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if all.since == 0 {
        all.since = now();
    }
    Mutex::new(Stats {
        started: now(),
        freed: 0,
        trims: 0,
        all,
        apps: HashMap::new(),
        events: VecDeque::new(),
        history: VecDeque::new(),
    })
});

/// The config the engine works from; the UI thread replaces it on every change.
static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static STOP: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn stats_path() -> std::path::PathBuf {
    config::dir().join("stats.json")
}

pub fn save_stats() {
    let all = STATS.lock().map(|s| s.all.clone());
    if let Ok(all) = all {
        if let Ok(text) = serde_json::to_string_pretty(&all) {
            let _ = std::fs::create_dir_all(config::dir());
            let _ = std::fs::write(stats_path(), text);
        }
    }
}

pub fn start(cfg: &Config) {
    apply(cfg);
    if let Ok(mut t) = THREAD.lock() {
        *t = Some(thread::spawn(run));
    }
}

/// Hands the engine a new config; it takes effect on the next poll.
pub fn apply(cfg: &Config) {
    if let Ok(mut c) = CONFIG.lock() {
        *c = Some(cfg.clone());
    }
}

/// Stops the thread (it lifts efficiency mode on its way out) and saves the stats.
pub fn shutdown() {
    STOP.store(true, Ordering::Relaxed);
    if let Some(t) = THREAD.lock().ok().and_then(|mut t| t.take()) {
        let _ = t.join();
    }
}

#[derive(Default)]
struct AppState {
    hidden_since: Option<Instant>,
    last_trim: HashMap<u32, Instant>,
    throttled: HashSet<u32>,
}

fn run() {
    let mut states: HashMap<String, AppState> = HashMap::new();
    let mut ticks = 0u64;
    while !STOP.load(Ordering::Relaxed) {
        let cfg = CONFIG.lock().ok().and_then(|c| c.clone());
        if let Some(cfg) = cfg {
            step(&cfg, &mut states);
        }
        ticks += 1;
        if ticks % 30 == 0 {
            save_stats(); // once a minute, so a crash loses little
        }
        let mut slept = Duration::ZERO;
        while slept < POLL && !STOP.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
            slept += Duration::from_millis(200);
        }
    }
    for st in states.values_mut() {
        restore(&mut st.throttled);
    }
    save_stats();
}

fn step(cfg: &Config, states: &mut HashMap<String, AppState>) {
    let snap = procs::snapshot();
    let mut total = 0u64;
    let mut seen = HashSet::new();
    for app in &cfg.apps {
        let key = app.key().to_string();
        seen.insert(key.clone());
        let st = states.entry(key.clone()).or_default();
        let pids: Vec<u32> = snap
            .iter()
            .filter(|(_, n)| app.exes.iter().any(|e| n.eq_ignore_ascii_case(e)))
            .map(|(p, _)| *p)
            .collect();
        st.throttled.retain(|p| pids.contains(p));
        st.last_trim.retain(|p, _| pids.contains(p));

        let shown = procs::any_window_shown(&pids);
        let active = !cfg.paused && app.mode != Mode::Paused;

        if active && app.eco && !shown {
            for &p in &pids {
                if st.throttled.insert(p) {
                    set_eco(p, true);
                }
            }
        } else {
            restore(&mut st.throttled);
        }

        let mut freed = None;
        if active {
            let every = if app.mode == Mode::Always { cfg.always_every_secs } else { cfg.hidden_every_secs };
            let interval = Duration::from_secs(every.max(1));
            let ready = match app.mode {
                Mode::Always => true,
                _ if shown => {
                    st.hidden_since = None;
                    st.last_trim.clear();
                    false
                }
                _ => st.hidden_since.get_or_insert_with(Instant::now).elapsed() >= interval,
            };
            if ready {
                for &p in &pids {
                    if st.last_trim.get(&p).map_or(true, |t| t.elapsed() >= interval) {
                        st.last_trim.insert(p, Instant::now());
                        if let Some(f) = trim_pid(p) {
                            *freed.get_or_insert(0) += f;
                        }
                    }
                }
            }
        }

        // Measured after trimming, so the number shown is what the app uses now.
        let ram: u64 = pids.iter().filter_map(|&p| procs::memory(p)).sum();
        total += ram;
        if let Ok(mut s) = STATS.lock() {
            let a = s.apps.entry(key).or_default();
            a.running = !pids.is_empty();
            a.ram = ram;
            a.peak = a.peak.max(ram);
            a.procs = pids.len();
            if let Some(f) = freed {
                record(&mut s, app, f);
            }
        }
    }

    // Apps the user removed: lift efficiency mode and forget them.
    states.retain(|k, st| {
        let keep = seen.contains(k);
        if !keep {
            restore(&mut st.throttled);
        }
        keep
    });
    if let Ok(mut s) = STATS.lock() {
        s.apps.retain(|k, _| seen.contains(k));
        s.history.push_back(total);
        while s.history.len() > HISTORY {
            s.history.pop_front();
        }
    }
}

/// Trims every process of an app right now ("Trim" buttons). Returns bytes freed.
pub fn trim_now(app: &AppCfg) -> u64 {
    let freed: u64 = procs::pids(&app.exes).into_iter().filter_map(trim_pid).sum();
    if let Ok(mut s) = STATS.lock() {
        record(&mut s, app, freed);
    }
    freed
}

/// "Trim all now": trims every app that isn't paused. Returns bytes freed.
pub fn trim_all(cfg: &Config) -> u64 {
    cfg.apps.iter().filter(|a| a.mode != Mode::Paused).map(trim_now).sum()
}

/// Trims one process. Returns bytes that left its working set, or None if it couldn't be opened.
fn trim_pid(pid: u32) -> Option<u64> {
    let h = procs::open(pid)?;
    let before = procs::working_set(h);
    unsafe { EmptyWorkingSet(h) };
    let after = procs::working_set(h);
    procs::close(h);
    Some(before.saturating_sub(after))
}

/// Adds one app trim (all its processes together) to every statistic.
fn record(s: &mut Stats, app: &AppCfg, freed: u64) {
    s.freed += freed;
    s.trims += 1;
    s.all.freed += freed;
    s.all.trims += 1;
    *s.all.per_app.entry(app.name.clone()).or_default() += freed;
    if freed > s.all.biggest {
        s.all.biggest = freed;
        s.all.biggest_app = app.name.clone();
    }
    let a = s.apps.entry(app.key().to_string()).or_default();
    a.freed += freed;
    a.trims += 1;
    a.last_trim = Some(now());
    // Skip near-zero trims so the activity list shows what mattered.
    if freed >= 1024 * 1024 {
        s.events.push_front(Event { at: now(), app: app.name.clone(), freed });
        s.events.truncate(EVENTS);
    }
}

fn restore(throttled: &mut HashSet<u32>) {
    for pid in throttled.drain() {
        set_eco(pid, false);
    }
}

fn set_eco(pid: u32, on: bool) {
    let Some(h) = procs::open(pid) else { return };
    let state = PROCESS_POWER_THROTTLING_STATE { Version: 1, ControlMask: 1, StateMask: on as u32 };
    unsafe {
        SetProcessInformation(h, ProcessPowerThrottling, &state as *const _ as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32);
    }
    procs::close(h);
}

/// (total, available) physical memory in bytes.
pub fn system_memory() -> (u64, u64) {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return (0, 0);
    }
    (m.ullTotalPhys, m.ullAvailPhys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_tracks_totals_biggest_and_events() {
        let app = AppCfg { name: "Test".into(), exes: vec!["test.exe".into()], mode: Mode::Always, eco: false };
        let mut s = Stats {
            started: 0, freed: 0, trims: 0, all: AllTime::default(),
            apps: HashMap::new(), events: VecDeque::new(), history: VecDeque::new(),
        };
        record(&mut s, &app, 5 * 1024 * 1024);
        record(&mut s, &app, 1000); // too small for the activity list
        assert_eq!((s.trims, s.all.trims), (2, 2));
        assert_eq!(s.freed, 5 * 1024 * 1024 + 1000);
        assert_eq!(s.all.biggest, 5 * 1024 * 1024);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.apps["test.exe"].trims, 2);
    }
}
