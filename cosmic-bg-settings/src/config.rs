// SPDX-License-Identifier: MPL-2.0

//! Configuration management for cosmic-bg-settings

use cosmic_bg_config::{Context, Entry, Source};

/// Load the current configuration from cosmic-config
pub fn load_config() -> Result<cosmic_bg_config::Config, cosmic_config::Error> {
    let context = cosmic_bg_config::context()?;
    cosmic_bg_config::Config::load(&context)
}

/// Save an entry to cosmic-config
pub fn save_entry(entry: Entry) -> Result<(), cosmic_config::Error> {
    let context = cosmic_bg_config::context()?;
    let mut config = cosmic_bg_config::Config::load(&context)?;
    config.set_entry(&context, entry)
}

/// Get the context for cosmic-config
/// TODO: Will be used for advanced configuration operations
#[allow(dead_code)]
pub fn get_context() -> Result<Context, cosmic_config::Error> {
    cosmic_bg_config::context()
}

/// Set whether all displays use the same wallpaper
/// TODO: Will be used when implementing per-output vs all-outputs toggle UI
#[allow(dead_code)]
pub fn set_same_on_all(value: bool) -> Result<(), cosmic_config::Error> {
    let context = cosmic_bg_config::context()?;
    context.set_same_on_all(value)
}

/// Get the default entry for all displays
/// TODO: Will be used when implementing output selection UI
#[allow(dead_code)]
pub fn get_default_entry() -> Result<Entry, cosmic_config::Error> {
    let context = cosmic_bg_config::context()?;
    Ok(context.default_background())
}

/// Get the entry for a specific output
/// TODO: Will be used when implementing per-output configuration UI
#[allow(dead_code)]
pub fn get_entry(output: &str) -> Result<Option<Entry>, cosmic_config::Error> {
    let context = cosmic_bg_config::context()?;
    match context.entry(output) {
        Ok(entry) => Ok(Some(entry)),
        Err(_) => Ok(None),
    }
}

/// Extract path from source if applicable
/// TODO: Will be used for file path operations and validation
#[allow(dead_code)]
pub fn source_path(source: &Source) -> Option<&std::path::Path> {
    match source {
        Source::Path(p) => Some(p.as_path()),
        Source::Video(v) => Some(v.path.as_path()),
        Source::Animated(a) => Some(a.path.as_path()),
        Source::Shader(s) => s.custom_path.as_deref(),
        Source::Color(_) => None,
    }
}
