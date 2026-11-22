//! Trims explorer.exe's working set. Explorer never hides (the taskbar and desktop stay up), so
//! the window-based trim in `trim.rs` doesn't fit. This runs two triggers:
//!   - Idle: once there has been no keyboard or mouse input for `explorer_idle_secs`, trim once
//!     per idle stretch.
//!   - Timer: every `explorer_interval_secs`, trim even while the user is active, since explorer
//!     refills quickly during use.
//! The timer can cause brief pauses when the Start menu or File Explorer pages memory back in.

use super::trim;
use super::Tweak;
use crate::config::Config;
use crate::procs;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

const POLL: Duration = Duration::from_secs(2);

/// Settings shared with the polling thread, so changes apply without a restart.
struct Timing {
    idle_secs: AtomicU64,
    interval_secs: AtomicU64,
}

pub struct ExplorerTrim {
    stop: Option<Arc<AtomicBool>>,
    timing: Arc<Timing>,
    thread: Option<JoinHandle<()>>,
}

impl ExplorerTrim {
    pub fn new() -> Self {
        let timing = Timing { idle_secs: AtomicU64::new(10), interval_secs: AtomicU64::new(60) };
        Self { stop: None, timing: Arc::new(timing), thread: None }
    }

    fn load_timing(&self, cfg: &Config) {
        self.timing.idle_secs.store(cfg.explorer_idle_secs, Ordering::Relaxed);
        self.timing.interval_secs.store(cfg.explorer_interval_secs, Ordering::Relaxed);
    }
}

impl Tweak for ExplorerTrim {
    fn id(&self) -> &'static str {
        "explorer_trim_idle"
    }
    fn name(&self) -> &'static str {
        "Trim Explorer memory"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn enable(&mut self, cfg: &Config) -> Result<(), String> {
        self.load_timing(cfg);
        let stop = Arc::new(AtomicBool::new(false));
        let (s, t) = (stop.clone(), self.timing.clone());
        self.thread = Some(thread::spawn(move || run(&s, &t)));
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

/// Time since the last keyboard or mouse input, for this user session.
fn idle_for() -> Duration {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return Duration::ZERO;
    }
    // GetTickCount wraps every ~49 days; wrapping_sub keeps the difference correct across a wrap.
    let ms = unsafe { GetTickCount() }.wrapping_sub(info.dwTime);
    Duration::from_millis(ms as u64)
}

/// Trims each explorer process on the idle trigger (once per idle stretch) or the timer.
fn run(stop: &AtomicBool, timing: &Timing) {
    let mut last_trim: HashMap<u32, Instant> = HashMap::new();
    let mut idle_trimmed: HashSet<u32> = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let idle = idle_for();
        let is_idle = idle >= Duration::from_secs(timing.idle_secs.load(Ordering::Relaxed));
        let interval = Duration::from_secs(timing.interval_secs.load(Ordering::Relaxed));
        if !is_idle {
            idle_trimmed.clear();
        }
        for pid in procs::pids(trim::EXPLORER_EXES) {
            let timer_due = last_trim.get(&pid).map_or(true, |t| t.elapsed() >= interval);
            // `insert` returns true only the first time this pid is seen in the current idle stretch.
            let idle_due = is_idle && idle_trimmed.insert(pid);
            if idle_due || timer_due {
                trim::trim(pid, "Windows Explorer");
                last_trim.insert(pid, Instant::now());
            }
        }
        sleep_unless_stopped(stop, POLL);
    }
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
