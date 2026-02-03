// SPDX-License-Identifier: MPL-2.0

//! Main application struct and logic for cosmic-bg-settings

use cosmic::app::Core;
use cosmic::iced::Length;
use cosmic::widget::{column, container, row, text};
use cosmic::{Action, Application, Element, Task};

use ashpd::desktop::file_chooser::{FileFilter, OpenFileRequest};

use crate::config;
use crate::message::{Message, SourceType};
use crate::pages::WallpaperPage;
use crate::widgets::PreviewWidget;

/// Application ID
pub const APP_ID: &str = "com.system76.CosmicBgSettings";

/// Main application state
pub struct App {
    /// Core application state from libcosmic
    core: Core,
    /// Current configuration
    config: Option<cosmic_bg_config::Config>,
    /// Wallpaper page state
    wallpaper_page: WallpaperPage,
    /// Preview widget
    preview: PreviewWidget,
}

impl Application for App {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Action<Self::Message>>) {
        let wallpaper_page = WallpaperPage::default();
        let preview = PreviewWidget::new();

        let app = Self {
            core,
            config: None,
            wallpaper_page,
            preview,
        };

        // Load configuration on startup
        let cmd = Task::perform(async { load_config_async() }, |config| {
            Action::App(Message::ConfigLoaded(config))
        });

        (app, cmd)
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![text::heading("Background Settings").into()]
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let page_view = self.wallpaper_page.view();

        // Build preview if we have a source
        let entry = self.wallpaper_page.build_entry();
        let preview_view = self.preview.view(&entry.source);

        let content = row()
            .spacing(16)
            .padding(16)
            .push(page_view)
            .push(
                column()
                    .spacing(8)
                    .push(text::title4("Preview"))
                    .push(preview_view),
            );

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message) -> Task<Action<Self::Message>> {
        let source_type = self.wallpaper_page.source_type;
        match message {
            Message::ConfigLoaded(config) => {
                self.wallpaper_page = WallpaperPage::new(&config);
                self.config = Some(config);
            }
            Message::ConfigChanged => {
                // Reload configuration
                return Task::perform(async { load_config_async() }, |config| {
                    Action::App(Message::ConfigLoaded(config))
                });
            }
            Message::SourceTypeChanged(source_type) => {
                self.wallpaper_page.source_type = source_type;
            }
            Message::FileSelected(path) => {
                self.wallpaper_page.selected_path = Some(path);
            }
            Message::OpenFilePicker => {
                return Task::perform(
                    async move { open_file_picker(source_type).await },
                    |result| match result {
                        Ok(path) => Action::App(Message::FileSelected(path)),
                        Err(_) => Action::App(Message::FilePickerCancelled),
                    },
                );
            }
            Message::FilePickerCancelled => {
                tracing::debug!("File picker cancelled");
            }
            Message::ShaderPresetChanged(preset) => {
                self.wallpaper_page.shader_preset = preset;
                self.wallpaper_page.custom_shader_path = None;
            }
            Message::CustomShaderSelected(path) => {
                self.wallpaper_page.custom_shader_path = Some(path);
            }
            Message::ColorSelected(rgb) => {
                self.wallpaper_page.primary_color = rgb;
            }
            Message::GradientColorsChanged(colors) => {
                self.wallpaper_page.gradient_colors = colors;
            }
            Message::GradientRadiusChanged(radius) => {
                self.wallpaper_page.gradient_radius = radius;
            }
            Message::ScalingModeChanged(mode) => {
                self.wallpaper_page.scaling_mode = mode;
            }
            Message::VideoLoopChanged(loop_) => {
                self.wallpaper_page.video_loop = loop_;
            }
            Message::VideoSpeedChanged(speed) => {
                self.wallpaper_page.video_speed = speed;
            }
            Message::VideoHwAccelChanged(hw_accel) => {
                self.wallpaper_page.video_hw_accel = hw_accel;
            }
            Message::AnimatedFpsChanged(fps) => {
                self.wallpaper_page.animated_fps = fps;
            }
            Message::AnimatedLoopCountChanged(count) => {
                self.wallpaper_page.animated_loop_count = count;
            }
            Message::ShaderFpsChanged(fps) => {
                self.wallpaper_page.shader_fps = fps;
            }
            Message::RotationFrequencyChanged(freq) => {
                self.wallpaper_page.rotation_frequency = freq;
            }
            Message::FilterByThemeChanged(filter) => {
                self.wallpaper_page.filter_by_theme = filter;
            }
            Message::OutputSelected(output) => {
                self.wallpaper_page.selected_output = output;
            }
            Message::ApplyToAllChanged(apply_to_all) => {
                self.wallpaper_page.apply_to_all = apply_to_all;
            }
            Message::Apply => {
                let entry = self.wallpaper_page.build_entry();
                return Task::perform(
                    async move {
                        if let Err(e) = config::save_entry(entry) {
                            tracing::error!("Failed to save entry: {}", e);
                        }
                    },
                    |_| Action::App(Message::ConfigChanged),
                );
            }
            Message::Cancel => {
                // Close the application
                std::process::exit(0);
            }
            Message::None => {}
        }

        Task::none()
    }
}

/// Open file picker dialog with filters based on source type
async fn open_file_picker(source_type: SourceType) -> Result<std::path::PathBuf, ashpd::Error> {
    // Create file filters based on source type
    let filters = create_file_filters(source_type);

    // Build the file chooser request
    let request = OpenFileRequest::default()
        .title(match source_type {
            SourceType::Static => "Select Image",
            SourceType::Video => "Select Video",
            SourceType::Animated => "Select Animated Image",
            SourceType::Shader => "Select Shader",
            _ => "Select File",
        })
        .accept_label("Select")
        .modal(true)
        .multiple(false);

    // Add filters
    let request = filters
        .into_iter()
        .fold(request, |req, filter| req.filter(filter));

    // Send the request
    let response = request.send().await?.response()?;

    // Get the selected file
    response
        .uris()
        .first()
        .and_then(|uri| uri.to_file_path().ok())
        .ok_or_else(|| ashpd::Error::NoResponse)
}

/// Create file filters based on source type
fn create_file_filters(source_type: SourceType) -> Vec<FileFilter> {
    match source_type {
        SourceType::Static => {
            vec![FileFilter::new("Images").glob("*.png").glob("*.jpg").glob("*.jpeg").glob("*.webp").glob("*.jxl")]
        }
        SourceType::Video => {
            vec![FileFilter::new("Videos").glob("*.mp4").glob("*.mkv").glob("*.webm").glob("*.avi").glob("*.mov")]
        }
        SourceType::Animated => {
            vec![FileFilter::new("Animated Images").glob("*.gif").glob("*.webp").glob("*.apng")]
        }
        SourceType::Shader => {
            vec![FileFilter::new("Shaders").glob("*.wgsl").glob("*.glsl")]
        }
        _ => vec![],
    }
}

/// Load configuration asynchronously
fn load_config_async() -> cosmic_bg_config::Config {
    config::load_config().unwrap_or_default()
}
