// SPDX-License-Identifier: MPL-2.0

//! Application messages for cosmic-bg-settings

use std::path::PathBuf;

use cosmic_bg_config::{ScalingMode, ShaderPreset};

/// Source type selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    Static,
    Video,
    Animated,
    Shader,
    Color,
    Gradient,
}

impl Default for SourceType {
    fn default() -> Self {
        Self::Static
    }
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static => write!(f, "Static Image"),
            Self::Video => write!(f, "Video"),
            Self::Animated => write!(f, "Animated Image"),
            Self::Shader => write!(f, "GPU Shader"),
            Self::Color => write!(f, "Solid Color"),
            Self::Gradient => write!(f, "Gradient"),
        }
    }
}

/// Application messages
#[derive(Debug, Clone)]
pub enum Message {
    /// Configuration loaded from cosmic-config
    ConfigLoaded(cosmic_bg_config::Config),

    /// Configuration changed externally
    ConfigChanged,

    /// Source type selection changed
    SourceTypeChanged(SourceType),

    /// File selected from file picker
    FileSelected(PathBuf),

    /// Open file picker dialog
    OpenFilePicker,

    /// File picker cancelled
    FilePickerCancelled,

    /// Shader preset selected
    ShaderPresetChanged(ShaderPreset),

    /// Custom shader path selected
    CustomShaderSelected(PathBuf),

    /// Color selected (RGB values 0-255)
    ColorSelected([u8; 3]),

    /// Gradient colors updated
    GradientColorsChanged(Vec<[u8; 3]>),

    /// Gradient radius changed
    GradientRadiusChanged(f32),

    /// Scaling mode changed
    ScalingModeChanged(ScalingMode),

    /// Video loop setting changed
    VideoLoopChanged(bool),

    /// Video playback speed changed
    VideoSpeedChanged(f64),

    /// Video hardware acceleration changed
    VideoHwAccelChanged(bool),

    /// Animated image FPS limit changed
    AnimatedFpsChanged(Option<u32>),

    /// Animated image loop count changed
    AnimatedLoopCountChanged(Option<u32>),

    /// Shader FPS limit changed
    ShaderFpsChanged(u32),

    /// Rotation frequency changed (for directories)
    RotationFrequencyChanged(u64),

    /// Filter by theme setting changed
    FilterByThemeChanged(bool),

    /// Output selection changed
    OutputSelected(String),

    /// Apply to all displays toggle changed
    ApplyToAllChanged(bool),

    /// Apply current settings
    Apply,

    /// Cancel and close
    Cancel,

    /// No-op message for placeholder events
    None,
}
