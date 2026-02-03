// SPDX-License-Identifier: MPL-2.0

use crate::{CosmicBg, CosmicBgLayer};
use crate::animated::AnimatedSource;
use crate::shader::ShaderSource;
use crate::source::WallpaperSource;
use crate::video::VideoSource;

use std::{
    collections::VecDeque,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use cosmic_bg_config::{Color, Entry, SamplingMethod, ScalingMode, Source, state::State};
use cosmic_config::CosmicConfigEntry;
use eyre::eyre;
use image::{DynamicImage, ImageReader};
use jxl_oxide::integration::JxlDecoder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rand::{rng, seq::SliceRandom};
use sctk::{
    reexports::{
        calloop::{
            self, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        client::QueueHandle,
    },
    shm::slot::CreateBufferError,
};
use thiserror::Error;
use tracing::error;
use walkdir::WalkDir;

// TODO filter images by whether they seem to match dark / light mode
// Alternatively only load from light / dark subdirectories given a directory source when this is active

#[derive(Debug, Error)]
pub enum DrawError {
    #[error("no source configured for wallpaper")]
    NoSource,
    #[error("failed to decode JPEG XL image: {0}")]
    JpegXlDecode(#[from] eyre::Report),
    #[error("failed to decode image from {path}: {reason}")]
    ImageDecode { path: PathBuf, reason: String },
    #[error("invalid color gradient in config")]
    InvalidGradient,
    #[error("failed to create buffer: {0}")]
    BufferCreation(#[from] CreateBufferError),
}

pub struct Wallpaper {
    pub entry: Entry,
    pub layers: Vec<CosmicBgLayer>,
    pub image_queue: VecDeque<PathBuf>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    queue_handle: QueueHandle<CosmicBg>,
    current_source: Option<Source>,
    // Cache of source image, if `current_source` is a `Source::Path`
    current_image: Option<image::DynamicImage>,
    timer_token: Option<RegistrationToken>,
    // Persistent animated source for videos/GIFs/shaders
    animated_source: Option<Box<dyn WallpaperSource>>,
    // Timer for animation frames
    animation_timer_token: Option<RegistrationToken>,
}

impl std::fmt::Debug for Wallpaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallpaper")
            .field("entry", &self.entry)
            .field("layers", &self.layers)
            .field("image_queue", &self.image_queue)
            .field("current_source", &self.current_source)
            .field("current_image", &self.current_image.as_ref().map(|_| "<DynamicImage>"))
            .field("timer_token", &self.timer_token)
            .field("animated_source", &self.animated_source.as_ref().map(|s| s.description()))
            .field("animation_timer_token", &self.animation_timer_token)
            .finish_non_exhaustive()
    }
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        if let Some(token) = self.timer_token.take() {
            self.loop_handle.remove(token);
        }
        if let Some(token) = self.animation_timer_token.take() {
            self.loop_handle.remove(token);
        }
    }
}

impl Wallpaper {
    pub fn new(
        entry: Entry,
        queue_handle: QueueHandle<CosmicBg>,
        loop_handle: calloop::LoopHandle<'static, CosmicBg>,
        source_tx: calloop::channel::SyncSender<(String, notify::Event)>,
    ) -> Self {
        let mut wallpaper = Wallpaper {
            entry,
            layers: Vec::new(),
            current_source: None,
            current_image: None,
            image_queue: VecDeque::default(),
            timer_token: None,
            animated_source: None,
            animation_timer_token: None,
            loop_handle,
            queue_handle,
        };

        wallpaper.load_images();
        wallpaper.register_timer();
        wallpaper.watch_source(source_tx);
        wallpaper
    }

    /// Update the wallpaper configuration without full recreation.
    ///
    /// This preserves the image cache and only updates changed settings,
    /// avoiding unnecessary file I/O and memory allocations.
    ///
    /// Currently unused - wallpapers are recreated on config changes. This method
    /// provides an optimization path for hot-reloading config without full teardown.
    #[allow(dead_code)]
    pub fn update_config(&mut self, new_entry: Entry) {
        let rotation_changed = self.entry.rotation_frequency != new_entry.rotation_frequency;
        let scaling_changed = self.entry.scaling_mode != new_entry.scaling_mode;
        let source_changed = self.entry.source != new_entry.source;

        tracing::debug!(
            output = %self.entry.output,
            rotation_changed,
            scaling_changed,
            source_changed,
            "Updating wallpaper config"
        );

        // Update the entry
        self.entry = new_entry;

        // If source changed, reload images (this will be called from apply_backgrounds)
        if source_changed {
            self.current_image = None;
            // Clear animated source and timer
            if let Some(token) = self.animation_timer_token.take() {
                self.loop_handle.remove(token);
            }
            self.animated_source = None;
            self.load_images();
        }

        // Re-register timer if rotation frequency changed
        if rotation_changed {
            if let Some(token) = self.timer_token.take() {
                self.loop_handle.remove(token);
            }
            self.register_timer();
        }

        // Trigger redraw if scaling mode changed
        if scaling_changed {
            for layer in &mut self.layers {
                layer.needs_redraw = true;
            }
        }
    }

    pub fn save_state(&self) -> Result<(), cosmic_config::Error> {
        let Some(cur_source) = self.current_source.clone() else {
            return Ok(());
        };
        let state_helper = State::state()?;
        let mut state = State::get_entry(&state_helper).unwrap_or_default();
        for l in &self.layers {
            let name = l.output_info.name.clone().unwrap_or_default();
            if let Some((_, source)) = state
                .wallpapers
                .iter_mut()
                .find(|(output, _)| *output == name)
            {
                *source = cur_source.clone();
            } else {
                state.wallpapers.push((name, cur_source.clone()))
            }
        }
        state.write_entry(&state_helper)
    }

    pub fn draw(&mut self) {
        let start = Instant::now();
        let mut cur_resized_img: Option<DynamicImage> = None;

        // Use indices to avoid borrow conflicts with self
        let layer_indices: Vec<usize> = self.layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.needs_redraw)
            .map(|(idx, _)| idx)
            .collect();

        for idx in layer_indices {
            match self.draw_layer_by_index(idx, &mut cur_resized_img, start) {
                Ok(()) => {}
                Err(DrawError::NoSource) => {
                    tracing::info!("No source for wallpaper");
                }
                Err(why) => {
                    tracing::error!(?why, "wallpaper could not be drawn");
                }
            }
        }
    }

    fn draw_layer_by_index(
        &mut self,
        layer_idx: usize,
        cur_resized_img: &mut Option<DynamicImage>,
        start: Instant,
    ) -> Result<(), DrawError> {
        // Calculate dimensions first (immutable borrow)
        let (width, height) = {
            let layer = self.layers.get(layer_idx).ok_or(DrawError::NoSource)?;
            self.calculate_layer_dimensions(layer)?
        };

        let needs_new_image = cur_resized_img
            .as_ref()
            .map_or(true, |img| img.width() != width || img.height() != height);

        if needs_new_image {
            *cur_resized_img = Some(self.prepare_scaled_image(width, height)?);
        }

        // Now we can get mutable access to the layer
        let layer = self.layers.get_mut(layer_idx).ok_or(DrawError::NoSource)?;
        let pool = layer.pool.as_mut().ok_or(DrawError::NoSource)?;

        let image = cur_resized_img.as_ref().expect("cur_resized_img was just set");

        let buffer = crate::draw::canvas(pool, image, width as i32, height as i32, width as i32 * 4)?;

        crate::draw::layer_surface(
            layer,
            &self.queue_handle,
            &buffer,
            (width as i32, height as i32),
        );

        layer.needs_redraw = false;

        let elapsed = Instant::now().duration_since(start);
        tracing::debug!(?elapsed, source = ?self.entry.source, "wallpaper draw");

        Ok(())
    }

    fn calculate_layer_dimensions(
        &self,
        layer: &CosmicBgLayer,
    ) -> Result<(u32, u32), DrawError> {
        let fractional_scale = layer.fractional_scale.ok_or(DrawError::NoSource)?;
        let (base_width, base_height) = layer.effective_size().ok_or(DrawError::NoSource)?;

        let width = base_width * fractional_scale / 120;
        let height = base_height * fractional_scale / 120;

        Ok((width, height))
    }

    fn prepare_scaled_image(&mut self, width: u32, height: u32) -> Result<DynamicImage, DrawError> {
        // Clone to avoid borrow conflicts when calling methods that mutate self
        let source = self.current_source.clone().ok_or(DrawError::NoSource)?;

        match source {
            Source::Path(ref path) => self.scale_image_from_path(path, width, height),
            Source::Color(Color::Single([r, g, b])) => {
                Ok(self.generate_solid_color([r, g, b], width, height))
            }
            Source::Color(Color::Gradient(ref gradient)) => {
                self.generate_gradient(gradient, width, height)
            }
            Source::Shader(_) | Source::Video(_) | Source::Animated(_) => {
                // Use persistent animated source
                let animated_source = self
                    .animated_source
                    .as_mut()
                    .ok_or_else(|| DrawError::ImageDecode {
                        path: PathBuf::from("animated"),
                        reason: "Animated source not initialized".to_string(),
                    })?;

                // Prepare with target dimensions if needed
                animated_source.prepare(width, height)
                    .map_err(|e| DrawError::ImageDecode {
                        path: PathBuf::from("animated"),
                        reason: format!("Failed to prepare animated source: {}", e),
                    })?;

                // Get the next frame
                let frame = animated_source.next_frame()
                    .map_err(|e| DrawError::ImageDecode {
                        path: PathBuf::from("animated"),
                        reason: format!("Failed to get next frame: {}", e),
                    })?;

                Ok(frame.image)
            }
        }
    }

    fn scale_image_from_path(
        &mut self,
        path: &Path,
        width: u32,
        height: u32,
    ) -> Result<DynamicImage, DrawError> {
        if self.current_image.is_none() {
            self.current_image = Some(self.decode_image(path)?);
        }

        let img = self.current_image.as_ref().expect("current_image was just set on line 190");
        Ok(self.apply_scaling_mode(img, width, height))
    }

    fn decode_image(&self, path: &Path) -> Result<DynamicImage, DrawError> {
        match path.extension() {
            Some(ext) if ext == "jxl" => decode_jpegxl(path).map_err(DrawError::from),
            _ => {
                let reader = ImageReader::open(path)
                    .and_then(|r| r.with_guessed_format())
                    .map_err(|e| DrawError::ImageDecode {
                        path: path.to_path_buf(),
                        reason: format!("failed to open image: {}", e),
                    })?;

                reader.decode().map_err(|e| DrawError::ImageDecode {
                    path: path.to_path_buf(),
                    reason: format!("failed to decode image: {}", e),
                })
            }
        }
    }

    fn apply_scaling_mode(&self, img: &DynamicImage, width: u32, height: u32) -> DynamicImage {
        match self.entry.scaling_mode {
            ScalingMode::Fit(color) => crate::scaler::fit(img, &color, width, height),
            ScalingMode::Zoom => crate::scaler::zoom(img, width, height),
            ScalingMode::Stretch => crate::scaler::stretch(img, width, height),
        }
    }

    fn generate_solid_color(&self, color: [f32; 3], width: u32, height: u32) -> DynamicImage {
        DynamicImage::from(crate::colored::single(color, width, height))
    }

    fn generate_gradient(
        &self,
        gradient: &cosmic_bg_config::Gradient,
        width: u32,
        height: u32,
    ) -> Result<DynamicImage, DrawError> {
        crate::colored::gradient(gradient, width, height)
            .map(DynamicImage::from)
            .map_err(|_| DrawError::InvalidGradient)
    }

    pub fn load_images(&mut self) {
        let mut image_queue = VecDeque::new();
        let xdg_data_dirs: Vec<String> = std::env::var("XDG_DATA_DIRS")
            .map(|dirs| dirs.split(':').map(|s| format!("{}/backgrounds/", s)).collect())
            .unwrap_or_default();

        match self.entry.source {
            Source::Path(ref source) => {
                tracing::debug!(?source, "loading images");

                if let Ok(source) = source.canonicalize() {
                    if source.is_dir() {
                        if xdg_data_dirs
                            .iter()
                            .any(|xdg_data_dir| source.starts_with(xdg_data_dir))
                        {
                            // Store paths of wallpapers to be used for the slideshow.
                            for img_path in WalkDir::new(source)
                                .follow_links(true)
                                .into_iter()
                                .filter_map(Result::ok)
                                .filter(|p| p.path().is_file())
                            {
                                image_queue.push_front(img_path.path().into());
                            }
                        } else if let Ok(dir) = source.read_dir() {
                            for entry in dir.filter_map(Result::ok) {
                                let Ok(path) = entry.path().canonicalize() else {
                                    continue;
                                };

                                if path.is_file() {
                                    image_queue.push_front(path);
                                }
                            }
                        }
                    } else if source.is_file() {
                        image_queue.push_front(source);
                    }
                }

                if image_queue.len() > 1 {
                    let image_slice = image_queue.make_contiguous();
                    match self.entry.sampling_method {
                        SamplingMethod::Alphanumeric => {
                            image_slice
                                .sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
                        }
                        SamplingMethod::Random => image_slice.shuffle(&mut rng()),
                    };

                    // If a wallpaper from this slideshow was previously set, resume with that wallpaper.
                    if let Some(Source::Path(last_path)) = current_image(&self.entry.output) {
                        if let Some(pos) = image_queue.iter().position(|p| p == &last_path) {
                            image_queue.rotate_left(pos);
                        }
                    }
                }

                image_queue.pop_front().map(|current_image_path| {
                    self.current_source = Some(Source::Path(current_image_path.clone()));
                    image_queue.push_back(current_image_path);
                });
            }

            Source::Color(ref c) => {
                self.current_source = Some(Source::Color(c.clone()));
            }

            Source::Shader(ref shader_config) => {
                // Shader wallpapers don't have image queues
                self.current_source = Some(Source::Shader(shader_config.clone()));

                // Create persistent shader source
                match ShaderSource::new(shader_config.clone()) {
                    Ok(shader_source) => {
                        self.animated_source = Some(Box::new(shader_source));
                        self.setup_animation_timer();
                    }
                    Err(e) => {
                        tracing::error!("Failed to create shader source: {}", e);
                    }
                }
            }

            Source::Video(ref video_config) => {
                // Video wallpapers don't have image queues
                self.current_source = Some(Source::Video(video_config.clone()));

                // Create persistent video source with converted config
                let video_cfg = crate::video::VideoConfig {
                    path: video_config.path.clone(),
                    loop_playback: video_config.loop_playback,
                    playback_speed: video_config.playback_speed,
                    hw_accel: video_config.hw_accel,
                };

                match VideoSource::new(video_cfg) {
                    Ok(video_source) => {
                        self.animated_source = Some(Box::new(video_source));
                        self.setup_animation_timer();
                    }
                    Err(e) => {
                        tracing::error!("Failed to create video source: {}", e);
                    }
                }
            }

            Source::Animated(ref animated_config) => {
                // Animated wallpapers don't have image queues
                self.current_source = Some(Source::Animated(animated_config.clone()));

                // Create persistent animated source with converted config
                let anim_cfg = crate::animated::AnimatedConfig {
                    path: animated_config.path.clone(),
                    fps_limit: animated_config.fps_limit,
                    loop_count: animated_config.loop_count,
                };

                match AnimatedSource::new(anim_cfg) {
                    Ok(animated_source) => {
                        self.animated_source = Some(Box::new(animated_source));
                        self.setup_animation_timer();
                    }
                    Err(e) => {
                        tracing::error!("Failed to create animated source: {}", e);
                    }
                }
            }
        };
        if let Err(err) = self.save_state() {
            error!("{err}");
        }
        self.image_queue = image_queue;
    }

    fn watch_source(&self, tx: calloop::channel::SyncSender<(String, notify::Event)>) {
        let Source::Path(ref source) = self.entry.source else {
            return;
        };

        let output = self.entry.output.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                if let Ok(e) = res {
                    let _ = tx.send((output.clone(), e));
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(_) => return,
        };

        tracing::debug!(output = self.entry.output, "watching source");

        if let Ok(m) = fs::metadata(source) {
            if m.is_dir() {
                let _ = watcher.watch(source, RecursiveMode::Recursive);
            } else if m.is_file() {
                let _ = watcher.watch(source, RecursiveMode::NonRecursive);
            }
        }
    }

    fn setup_animation_timer(&mut self) {
        // Remove existing animation timer if present
        if let Some(token) = self.animation_timer_token.take() {
            self.loop_handle.remove(token);
        }

        // Get frame duration from the animated source
        let frame_duration = self
            .animated_source
            .as_ref()
            .map(|source| source.frame_duration())
            .unwrap_or(Duration::from_millis(33)); // Default to ~30fps

        let output = self.entry.output.clone();

        // Register continuous animation timer
        self.animation_timer_token = self
            .loop_handle
            .insert_source(
                Timer::from_duration(frame_duration),
                move |_, _, state: &mut CosmicBg| {
                    let span = tracing::debug_span!("Wallpaper::animation_timer");
                    let _handle = span.enter();

                    let Some(item) = state
                        .wallpapers
                        .iter_mut()
                        .find(|w| w.entry.output == output)
                    else {
                        return TimeoutAction::Drop; // Drop if no item found
                    };

                    // Trigger redraw for animated content
                    for layer in &mut item.layers {
                        layer.needs_redraw = true;
                    }
                    item.draw();

                    // Get updated frame duration from source
                    let next_duration = item
                        .animated_source
                        .as_ref()
                        .map(|source| source.frame_duration())
                        .unwrap_or(Duration::from_millis(33));

                    TimeoutAction::ToDuration(next_duration)
                },
            )
            .ok();
    }

    fn register_timer(&mut self) {
        let rotation_freq = self.entry.rotation_frequency;
        let output = self.entry.output.clone();
        // set timer for rotation
        if rotation_freq > 0 {
            self.timer_token = self
                .loop_handle
                .insert_source(
                    Timer::from_duration(Duration::from_secs(rotation_freq)),
                    move |_, _, state: &mut CosmicBg| {
                        let span = tracing::debug_span!("Wallpaper::timer");
                        let _handle = span.enter();

                        let Some(item) = state
                            .wallpapers
                            .iter_mut()
                            .find(|w| w.entry.output == output)
                        else {
                            return TimeoutAction::Drop; // Drop if no item found for this timer
                        };

                        while let Some(next) = item.image_queue.pop_front() {
                            item.current_source = Some(Source::Path(next.clone()));
                            if let Err(err) = item.save_state() {
                                error!("{err}");
                            }

                            item.image_queue.push_back(next);
                            item.clear_image();
                            item.draw();

                            return TimeoutAction::ToDuration(Duration::from_secs(rotation_freq));
                        }

                        TimeoutAction::Drop
                    },
                )
                .ok();
        }
    }

    fn clear_image(&mut self) {
        self.current_image = None;
        for l in &mut self.layers {
            l.needs_redraw = true;
        }
    }
}

fn current_image(output: &str) -> Option<Source> {
    let state = State::state().ok()?;
    let mut wallpapers = State::get_entry(&state)
        .unwrap_or_default()
        .wallpapers
        .into_iter();

    let wallpaper = if output == "all" {
        wallpapers.next()
    } else {
        wallpapers.find(|(name, _path)| name == output)
    };

    wallpaper.map(|(_name, path)| path)
}

/// Decodes JPEG XL image files into `image::DynamicImage` via `jxl-oxide`.
fn decode_jpegxl(path: &std::path::Path) -> eyre::Result<DynamicImage> {
    let file = File::open(path).map_err(|why| eyre!("failed to open jxl image file: {why}"))?;

    let decoder =
        JxlDecoder::new(file).map_err(|why| eyre!("failed to read jxl image header: {why}"))?;

    image::DynamicImage::from_decoder(decoder)
        .map_err(|why| eyre!("failed to decode jxl image: {why}"))
}
