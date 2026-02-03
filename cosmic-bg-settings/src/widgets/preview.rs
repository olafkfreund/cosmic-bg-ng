// SPDX-License-Identifier: MPL-2.0

//! Wallpaper preview widget

use cosmic::iced::Length;
use cosmic::widget::{column, container, text};
use cosmic::Element;
use cosmic_bg_config::{Color, Source};

use crate::message::Message;

/// Widget for previewing wallpapers
#[derive(Debug, Clone, Default)]
pub struct PreviewWidget {
    /// Cached image path for static images
    _image_path: Option<std::path::PathBuf>,
}

impl PreviewWidget {
    /// Create a new preview widget
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the view for this widget
    pub fn view(&self, source: &Source) -> Element<'_, Message> {
        let preview_content: Element<'_, Message> = match source {
            Source::Path(path) if path.exists() => {
                if path.is_file() {
                    // Show file path (image preview would require loading)
                    column()
                        .push(text::body("Image:"))
                        .push(text::caption(
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ))
                        .into()
                } else {
                    // Directory
                    column()
                        .push(text::body("Directory:"))
                        .push(text::caption(path.display().to_string()))
                        .into()
                }
            }
            Source::Video(config) if config.path.exists() => {
                column()
                    .push(text::body("Video:"))
                    .push(text::caption(
                        config
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ))
                    .into()
            }
            Source::Animated(config) if config.path.exists() => {
                column()
                    .push(text::body("Animated:"))
                    .push(text::caption(
                        config
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ))
                    .into()
            }
            Source::Shader(config) => {
                let name = if let Some(preset) = &config.preset {
                    format!("Shader: {:?}", preset)
                } else if let Some(path) = &config.custom_path {
                    format!(
                        "Custom: {}",
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    )
                } else {
                    "Unknown shader".to_string()
                };
                text::body(name).into()
            }
            Source::Color(color) => match color {
                Color::Single(rgb) => {
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (rgb[0] * 255.0) as u8,
                        (rgb[1] * 255.0) as u8,
                        (rgb[2] * 255.0) as u8
                    );
                    text::body(format!("Solid color: {hex}")).into()
                }
                Color::Gradient(g) => {
                    let colors: Vec<String> = g
                        .colors
                        .iter()
                        .map(|c| {
                            format!(
                                "#{:02x}{:02x}{:02x}",
                                (c[0] * 255.0) as u8,
                                (c[1] * 255.0) as u8,
                                (c[2] * 255.0) as u8
                            )
                        })
                        .collect();
                    text::body(format!("Gradient: {}", colors.join(" -> "))).into()
                }
            },
            _ => text::body("No preview available").into(),
        };

        container(preview_content)
            .width(Length::Fixed(320.0))
            .height(Length::Fixed(240.0))
            .into()
    }
}
