//! Puts Discord's processes into Windows efficiency mode (EcoQoS) while none of its windows
//! are shown, and takes it off again as soon as one is. Efficiency mode only lowers CPU
//! scheduling and power use. It does not change what Discord does, and it is removed
//! right away when a window is restored.

use super::Tweak;
use crate::config::Config;
use crate::procs;
use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    ProcessPowerThrottling, SetProcessInformation, PROCESS_POWER_THROTTLING_STATE,
};

pub struct EcoQos {
    stop: Option<Arc<AtomicBool>>,
    thread: Option<JoinHandle<()>>,
}

impl EcoQos {
    pub fn new() -> Self {
        Self { stop: None, thread: None }
    }
}

impl Tweak for EcoQos {
    fn id(&self) -> &'static str {
        "discord_eco_qos"
    }
    fn name(&self) -> &'static str {
        "Efficiency mode for hidden Discord"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn enable(&mut self, _cfg: &Config) -> Result<(), String> {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        self.thread = Some(thread::spawn(move || run(&s)));
        self.stop = Some(stop);
        Ok(())
    }
    fn disable(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        self.thread.take();
    }
    fn is_enabled(&self) -> bool {
        self.stop.is_some()
    }
}

/// While Discord has no shown window, throttles every Discord process it can see. Any process
/// that was throttled is restored as soon as a window shows, and again when the feature stops.
fn run(stop: &AtomicBool) {
    let mut throttled: HashSet<u32> = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let pids = procs::pids(DISCORD_EXES);
        if procs::any_window_shown(&pids) {
            restore(&mut throttled);
        } else {
            for pid in pids {
                if throttled.insert(pid) {
                    set_eco(pid, true);
                }
            }
        }
        sleep_unless_stopped(stop, POLL);
    }
    restore(&mut throttled);
}

const POLL: Duration = Duration::from_secs(2);
const DISCORD_EXES: &[&str] = &["discord.exe"];

fn restore(throttled: &mut HashSet<u32>) {
    for pid in throttled.drain() {
        set_eco(pid, false);
    }
}

fn set_eco(pid: u32, on: bool) {
    let Some(h) = procs::open(pid) else { return };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: 1,
        ControlMask: 1,
        StateMask: if on { 1 } else { 0 },
    };
    set_info(h, &state);
    procs::close(h);
}

fn set_info(h: HANDLE, state: &PROCESS_POWER_THROTTLING_STATE) {
    unsafe {
        SetProcessInformation(
            h,
            ProcessPowerThrottling,
            state as *const PROCESS_POWER_THROTTLING_STATE as *const c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
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
