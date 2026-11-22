//! Every feature implements `Tweak`. To add one:
//!   1. create `src/tweaks/my_feature.rs` with a struct implementing `Tweak`
//!   2. add `pub mod my_feature;` below and push it in `all()`
//!
//! It then shows up in the tray menu and is saved in config.json automatically.

use crate::config::Config;

pub mod explorer;
pub mod throttle;
pub mod trim;

pub trait Tweak {
    /// Stable id used as the key in config.json. Never change it once shipped.
    fn id(&self) -> &'static str;
    /// Label shown in the tray menu.
    fn name(&self) -> &'static str;
    /// Whether the feature is on when the config has no entry for it yet.
    fn default_enabled(&self) -> bool {
        false
    }
    fn enable(&mut self, cfg: &Config) -> Result<(), String>;
    fn disable(&mut self);
    fn is_enabled(&self) -> bool;
    /// Called after settings change while the feature is running.
    fn apply_config(&mut self, _cfg: &Config) {}
}

pub fn all() -> Vec<Box<dyn Tweak>> {
    vec![
        Box::new(trim::TrimIdle::new(trim::DISCORD)),
        Box::new(trim::TrimIdle::new_active(trim::DISCORD_ACTIVE)),
        Box::new(trim::TrimIdle::new(trim::STEAM)),
        Box::new(trim::TrimIdle::new(trim::CTF_LOADER)),
        Box::new(explorer::ExplorerTrim::new()),
        Box::new(throttle::EcoQos::new()),
    ]
}
