// SPDX-License-Identifier: MPL-2.0

//! Main wallpaper configuration page

use std::path::PathBuf;

use cosmic::iced::Length;
use cosmic::widget::{button, column, container, dropdown, row, text, text_input, toggler};
use cosmic::Element;
use cosmic_bg_config::{
    AnimatedConfig, Color, Entry, Gradient, ScalingMode, ShaderConfig, ShaderPreset, Source,
    VideoConfig,
};

use crate::message::{Message, SourceType};

/// Source type dropdown options
static SOURCE_TYPE_NAMES: &[&str] = &[
    "Static Image",
    "Video",
    "Animated Image",
    "GPU Shader",
    "Solid Color",
    "Gradient",
];

/// Scaling mode dropdown options
static SCALING_MODE_NAMES: &[&str] = &["Zoom (fill)", "Fit (letterbox)", "Stretch"];

/// Shader preset dropdown options
static SHADER_PRESET_NAMES: &[&str] = &["Plasma", "Waves", "Gradient"];

/// State for the wallpaper configuration page
#[derive(Debug, Clone)]
pub struct WallpaperPage {
    /// Currently selected source type
    pub source_type: SourceType,
    /// Selected file path (for static, video, animated)
    pub selected_path: Option<PathBuf>,
    /// Selected shader preset
    pub shader_preset: ShaderPreset,
    /// Custom shader path
    pub custom_shader_path: Option<PathBuf>,
    /// Current scaling mode
    pub scaling_mode: ScalingMode,
    /// Video loop setting
    pub video_loop: bool,
    /// Video playback speed
    pub video_speed: f64,
    /// Video hardware acceleration
    pub video_hw_accel: bool,
    /// Animated FPS limit
    pub animated_fps: Option<u32>,
    /// Animated loop count
    pub animated_loop_count: Option<u32>,
    /// Shader FPS limit
    pub shader_fps: u32,
    /// Rotation frequency for directories
    pub rotation_frequency: u64,
    /// Filter by theme
    pub filter_by_theme: bool,
    /// Selected output
    pub selected_output: String,
    /// Apply to all displays
    pub apply_to_all: bool,
    /// Primary color (for solid color/gradient)
    pub primary_color: [u8; 3],
    /// Gradient colors
    pub gradient_colors: Vec<[u8; 3]>,
    /// Gradient radius
    pub gradient_radius: f32,
    /// Available outputs
    pub available_outputs: Vec<String>,
    /// Selected source type index for dropdown
    source_type_idx: usize,
    /// Selected scaling mode index for dropdown
    scaling_mode_idx: usize,
    /// Selected shader preset index for dropdown
    shader_preset_idx: usize,
}

impl Default for WallpaperPage {
    fn default() -> Self {
        Self {
            source_type: SourceType::Static,
            selected_path: None,
            shader_preset: ShaderPreset::Plasma,
            custom_shader_path: None,
            scaling_mode: ScalingMode::Zoom,
            video_loop: true,
            video_speed: 1.0,
            video_hw_accel: true,
            animated_fps: None,
            animated_loop_count: None,
            shader_fps: 30,
            rotation_frequency: 900,
            filter_by_theme: false,
            selected_output: "all".to_string(),
            apply_to_all: true,
            primary_color: [0, 0, 0],
            gradient_colors: vec![[0, 0, 128], [128, 0, 128]],
            gradient_radius: 0.5,
            available_outputs: vec!["all".to_string()],
            source_type_idx: 0,
            scaling_mode_idx: 0,
            shader_preset_idx: 0,
        }
    }
}

impl WallpaperPage {
    /// Create a new wallpaper page with initial config
    pub fn new(config: &cosmic_bg_config::Config) -> Self {
        let mut page = Self::default();
        page.apply_to_all = config.same_on_all;
        page.load_from_entry(&config.default_background);
        page
    }

    /// Load settings from an entry
    pub fn load_from_entry(&mut self, entry: &Entry) {
        self.selected_output = entry.output.clone();
        self.scaling_mode = entry.scaling_mode.clone();
        self.rotation_frequency = entry.rotation_frequency;
        self.filter_by_theme = entry.filter_by_theme;

        // Update scaling mode index
        self.scaling_mode_idx = match &self.scaling_mode {
            ScalingMode::Zoom => 0,
            ScalingMode::Fit(_) => 1,
            ScalingMode::Stretch => 2,
        };

        match &entry.source {
            Source::Path(path) => {
                self.source_type = SourceType::Static;
                self.source_type_idx = 0;
                self.selected_path = Some(path.clone());
            }
            Source::Video(config) => {
                self.source_type = SourceType::Video;
                self.source_type_idx = 1;
                self.selected_path = Some(config.path.clone());
                self.video_loop = config.loop_playback;
                self.video_speed = config.playback_speed;
                self.video_hw_accel = config.hw_accel;
            }
            Source::Animated(config) => {
                self.source_type = SourceType::Animated;
                self.source_type_idx = 2;
                self.selected_path = Some(config.path.clone());
                self.animated_fps = config.fps_limit;
                self.animated_loop_count = config.loop_count;
            }
            Source::Shader(config) => {
                self.source_type = SourceType::Shader;
                self.source_type_idx = 3;
                if let Some(preset) = &config.preset {
                    self.shader_preset = preset.clone();
                    self.shader_preset_idx = match preset {
                        ShaderPreset::Plasma => 0,
                        ShaderPreset::Waves => 1,
                        ShaderPreset::Gradient => 2,
                    };
                    self.custom_shader_path = None;
                } else {
                    self.custom_shader_path = config.custom_path.clone();
                }
                self.shader_fps = config.fps_limit;
            }
            Source::Color(color) => match color {
                Color::Single(rgb) => {
                    self.source_type = SourceType::Color;
                    self.source_type_idx = 4;
                    self.primary_color = [
                        (rgb[0] * 255.0) as u8,
                        (rgb[1] * 255.0) as u8,
                        (rgb[2] * 255.0) as u8,
                    ];
                }
                Color::Gradient(gradient) => {
                    self.source_type = SourceType::Gradient;
                    self.source_type_idx = 5;
                    self.gradient_colors = gradient
                        .colors
                        .iter()
                        .map(|c| [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8])
                        .collect();
                    self.gradient_radius = gradient.radius;
                }
            },
        }
    }

    /// Build the current entry from page state
    pub fn build_entry(&self) -> Entry {
        let source = match self.source_type {
            SourceType::Static => Source::Path(self.selected_path.clone().unwrap_or_default()),
            SourceType::Video => Source::Video(VideoConfig {
                path: self.selected_path.clone().unwrap_or_default(),
                loop_playback: self.video_loop,
                playback_speed: self.video_speed,
                hw_accel: self.video_hw_accel,
            }),
            SourceType::Animated => Source::Animated(AnimatedConfig {
                path: self.selected_path.clone().unwrap_or_default(),
                fps_limit: self.animated_fps,
                loop_count: self.animated_loop_count,
            }),
            SourceType::Shader => Source::Shader(ShaderConfig {
                preset: if self.custom_shader_path.is_none() {
                    Some(self.shader_preset.clone())
                } else {
                    None
                },
                custom_path: self.custom_shader_path.clone(),
                fps_limit: self.shader_fps,
            }),
            SourceType::Color => Source::Color(Color::Single([
                self.primary_color[0] as f32 / 255.0,
                self.primary_color[1] as f32 / 255.0,
                self.primary_color[2] as f32 / 255.0,
            ])),
            SourceType::Gradient => Source::Color(Color::Gradient(Gradient {
                colors: self
                    .gradient_colors
                    .iter()
                    .map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0])
                    .collect::<Vec<_>>()
                    .into(),
                radius: self.gradient_radius,
            })),
        };

        let mut entry = Entry::new(
            if self.apply_to_all {
                "all".to_string()
            } else {
                self.selected_output.clone()
            },
            source,
        );
        entry.scaling_mode = self.scaling_mode.clone();
        entry.rotation_frequency = self.rotation_frequency;
        entry.filter_by_theme = self.filter_by_theme;
        entry
    }

    /// Build the view for this page
    pub fn view(&self) -> Element<'_, Message> {
        // Source type dropdown
        let source_dropdown = dropdown(SOURCE_TYPE_NAMES, Some(self.source_type_idx), |idx| {
            let source_type = match idx {
                0 => SourceType::Static,
                1 => SourceType::Video,
                2 => SourceType::Animated,
                3 => SourceType::Shader,
                4 => SourceType::Color,
                5 => SourceType::Gradient,
                _ => SourceType::Static,
            };
            Message::SourceTypeChanged(source_type)
        })
        .width(Length::Fixed(200.0));

        // Build source-specific options
        let source_options: Element<'_, Message> = match self.source_type {
            SourceType::Static | SourceType::Video | SourceType::Animated => {
                let path_text = self
                    .selected_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "No file selected".to_string());

                let browse_btn =
                    button::standard("Browse...").on_press(Message::OpenFilePicker);

                row()
                    .spacing(8)
                    .push(text::body(path_text))
                    .push(browse_btn)
                    .into()
            }
            SourceType::Shader => {
                let preset_dropdown =
                    dropdown(SHADER_PRESET_NAMES, Some(self.shader_preset_idx), |idx| {
                        let preset = match idx {
                            0 => ShaderPreset::Plasma,
                            1 => ShaderPreset::Waves,
                            2 => ShaderPreset::Gradient,
                            _ => ShaderPreset::Plasma,
                        };
                        Message::ShaderPresetChanged(preset)
                    })
                    .width(Length::Fixed(150.0));

                let fps_input = text_input("FPS", self.shader_fps.to_string())
                    .on_input(|s| {
                        s.parse::<u32>()
                            .map(Message::ShaderFpsChanged)
                            .unwrap_or(Message::None)
                    })
                    .width(Length::Fixed(80.0));

                column()
                    .spacing(8)
                    .push(row().spacing(8).push(text::body("Preset:")).push(preset_dropdown))
                    .push(row().spacing(8).push(text::body("FPS Limit:")).push(fps_input))
                    .into()
            }
            SourceType::Color => {
                let color_hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    self.primary_color[0], self.primary_color[1], self.primary_color[2]
                );

                let color_input = text_input("Color", color_hex)
                    .on_input(|s| parse_color_input(&s))
                    .width(Length::Fixed(100.0));

                row()
                    .spacing(8)
                    .push(text::body("Color:"))
                    .push(color_input)
                    .into()
            }
            SourceType::Gradient => {
                let colors_text = self
                    .gradient_colors
                    .iter()
                    .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
                    .collect::<Vec<_>>()
                    .join(", ");

                let radius_input = text_input("Radius", format!("{:.2}", self.gradient_radius))
                    .on_input(|s| {
                        s.parse::<f32>()
                            .map(Message::GradientRadiusChanged)
                            .unwrap_or(Message::None)
                    })
                    .width(Length::Fixed(80.0));

                column()
                    .spacing(8)
                    .push(row().spacing(8).push(text::body("Colors:")).push(text::body(colors_text)))
                    .push(row().spacing(8).push(text::body("Radius:")).push(radius_input))
                    .into()
            }
        };

        // Video-specific options
        let video_options: Element<'_, Message> = if self.source_type == SourceType::Video {
            let speed_input = text_input("Speed", format!("{:.1}", self.video_speed))
                .on_input(|s| {
                    s.parse::<f64>()
                        .map(Message::VideoSpeedChanged)
                        .unwrap_or(Message::None)
                })
                .width(Length::Fixed(80.0));

            column()
                .spacing(8)
                .push(
                    row()
                        .spacing(8)
                        .push(text::body("Loop:"))
                        .push(toggler(self.video_loop).on_toggle(Message::VideoLoopChanged)),
                )
                .push(row().spacing(8).push(text::body("Speed:")).push(speed_input))
                .push(
                    row()
                        .spacing(8)
                        .push(text::body("HW Accel:"))
                        .push(toggler(self.video_hw_accel).on_toggle(Message::VideoHwAccelChanged)),
                )
                .into()
        } else {
            column().into()
        };

        // Animated-specific options
        let animated_options: Element<'_, Message> = if self.source_type == SourceType::Animated {
            let fps_input = text_input(
                "FPS",
                self.animated_fps
                    .map(|f| f.to_string())
                    .unwrap_or_default(),
            )
            .on_input(|s| {
                if s.is_empty() {
                    Message::AnimatedFpsChanged(None)
                } else {
                    s.parse::<u32>()
                        .map(|v| Message::AnimatedFpsChanged(Some(v)))
                        .unwrap_or(Message::None)
                }
            })
            .width(Length::Fixed(80.0));

            let loops_input = text_input(
                "Loops",
                self.animated_loop_count
                    .map(|l| l.to_string())
                    .unwrap_or_default(),
            )
            .on_input(|s| {
                if s.is_empty() {
                    Message::AnimatedLoopCountChanged(None)
                } else {
                    s.parse::<u32>()
                        .map(|v| Message::AnimatedLoopCountChanged(Some(v)))
                        .unwrap_or(Message::None)
                }
            })
            .width(Length::Fixed(80.0));

            column()
                .spacing(8)
                .push(row().spacing(8).push(text::body("FPS Limit:")).push(fps_input))
                .push(row().spacing(8).push(text::body("Loop Count:")).push(loops_input))
                .into()
        } else {
            column().into()
        };

        // Scaling mode dropdown
        let scaling_dropdown =
            dropdown(SCALING_MODE_NAMES, Some(self.scaling_mode_idx), |idx| {
                let mode = match idx {
                    0 => ScalingMode::Zoom,
                    1 => ScalingMode::Fit([0.0, 0.0, 0.0]),
                    2 => ScalingMode::Stretch,
                    _ => ScalingMode::Zoom,
                };
                Message::ScalingModeChanged(mode)
            })
            .width(Length::Fixed(150.0));

        // Rotation frequency input
        let rotation_input = text_input("Rotation", self.rotation_frequency.to_string())
            .on_input(|s| {
                s.parse::<u64>()
                    .map(Message::RotationFrequencyChanged)
                    .unwrap_or(Message::None)
            })
            .width(Length::Fixed(80.0));

        // Action buttons
        let apply_btn = button::suggested("Apply").on_press(Message::Apply);
        let cancel_btn = button::standard("Cancel").on_press(Message::Cancel);

        // Build the main layout
        let content = column()
            .spacing(12)
            .padding(16)
            .push(text::title4("Source Type"))
            .push(source_dropdown)
            .push(source_options)
            .push(video_options)
            .push(animated_options)
            .push(text::title4("Scaling"))
            .push(scaling_dropdown)
            .push(
                row()
                    .spacing(8)
                    .push(text::body("Rotation (seconds):"))
                    .push(rotation_input),
            )
            .push(
                row()
                    .spacing(8)
                    .push(text::body("Filter by theme:"))
                    .push(toggler(self.filter_by_theme).on_toggle(Message::FilterByThemeChanged)),
            )
            .push(text::title4("Display"))
            .push(
                row()
                    .spacing(8)
                    .push(text::body("Apply to all displays:"))
                    .push(toggler(self.apply_to_all).on_toggle(Message::ApplyToAllChanged)),
            )
            .push(row().spacing(8).push(apply_btn).push(cancel_btn));

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// Parse color input string to message
fn parse_color_input(input: &str) -> Message {
    let hex = input.trim_start_matches('#');
    if hex.len() != 6 {
        return Message::None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok();
    let g = u8::from_str_radix(&hex[2..4], 16).ok();
    let b = u8::from_str_radix(&hex[4..6], 16).ok();

    match (r, g, b) {
        (Some(r), Some(g), Some(b)) => Message::ColorSelected([r, g, b]),
        _ => Message::None,
    }
}
