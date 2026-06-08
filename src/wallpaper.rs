// SPDX-License-Identifier: MPL-2.0

use crate::{CosmicBg, CosmicBgLayer};
use crate::animated::AnimatedSource;
use crate::scaler::FitBlurBgrxWorkspace;
use crate::shader::ShaderSource;
use crate::source::{FramePayload, WallpaperSource};
use crate::video::VideoSource;

use std::{
    collections::VecDeque,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use cosmic_ext_bg_config::{Color, Entry, SamplingMethod, ScalingMode, Source, state::State};
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
        client::{QueueHandle, protocol::wl_surface},
    },
    shell::WaylandSurface,
    shm::slot::CreateBufferError,
};
use thiserror::Error;
use tracing::error;
use walkdir::WalkDir;

const FIT_BLUR_BACKGROUND_REFRESH_FRAMES: u32 = 60;

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
    // Filesystem watcher for live wallpaper directory updates.
    // Must be stored here to keep the watcher alive for the lifetime of this wallpaper.
    _watcher: Option<RecommendedWatcher>,
    fit_blur_background_cache: Vec<FitBlurBackgroundCache>,
    fit_blur_bgrx_workspace: FitBlurBgrxWorkspace,
}

struct FitBlurBackgroundCache {
    layer_width: u32,
    layer_height: u32,
    frame_width: u32,
    frame_height: u32,
    frames_since_refresh: u32,
    background: Arc<[u8]>,
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
            .field("fit_blur_background_cache", &self.fit_blur_background_cache.len())
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

enum PreparedVideoFrame {
    Bgrx {
        data: Arc<[u8]>,
        width: u32,
        height: u32,
        stride: usize,
    },
    FitBlurBgrx {
        background: Arc<[u8]>,
        data: Arc<[u8]>,
        frame_width: u32,
        frame_height: u32,
        frame_stride: usize,
        layer_width: u32,
        layer_height: u32,
    },
    Image(DynamicImage),
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
            _watcher: None,
            fit_blur_background_cache: Vec::new(),
            fit_blur_bgrx_workspace: FitBlurBgrxWorkspace::default(),
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
            self.fit_blur_background_cache.clear();
            // Clear animated source and timer
            if let Some(token) = self.animation_timer_token.take() {
                self.loop_handle.remove(token);
            }
            self.animated_source = None;
            for layer in &mut self.layers {
                layer.last_video_frame_draw = None;
            }
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
            self.fit_blur_background_cache.clear();
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
        let mut cur_resized_img: Option<Arc<DynamicImage>> = None;

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

    pub fn draw_video_frame_for_surface(&mut self, surface: &wl_surface::WlSurface) -> bool {
        if !matches!(self.entry.source, Source::Video(_)) {
            return false;
        }

        let frame_duration = self
            .animated_source
            .as_ref()
            .map(|source| source.frame_duration())
            .unwrap_or(crate::source::DEFAULT_FRAME_DURATION);

        let Some(layer_idx) = self
            .layers
            .iter()
            .position(|layer| layer.layer.wl_surface() == surface)
        else {
            return false;
        };

        if self.layers[layer_idx]
            .last_video_frame_draw
            .is_some_and(|last_draw| last_draw.elapsed() < frame_duration)
        {
            self.layers[layer_idx]
                .layer
                .wl_surface()
                .frame(&self.queue_handle, surface.clone());
            surface.commit();
            return true;
        }

        self.layers[layer_idx].needs_redraw = true;

        match self.draw_layer_by_index(layer_idx, &mut None, Instant::now()) {
            Ok(()) => {
                self.layers[layer_idx].last_video_frame_draw = Some(Instant::now());
            }
            Err(DrawError::NoSource) => {
                tracing::info!("No source for wallpaper");
                self.layers[layer_idx]
                    .layer
                    .wl_surface()
                    .frame(&self.queue_handle, surface.clone());
                surface.commit();
            }
            Err(why) => {
                tracing::error!(?why, "wallpaper could not be drawn");
            }
        }

        true
    }

    fn draw_layer_by_index(
        &mut self,
        layer_idx: usize,
        cur_resized_img: &mut Option<Arc<DynamicImage>>,
        start: Instant,
    ) -> Result<(), DrawError> {
        // Calculate dimensions first (immutable borrow)
        let (width, height) = {
            let layer = self.layers.get(layer_idx).ok_or(DrawError::NoSource)?;
            self.calculate_layer_dimensions(layer)?
        };

        if !matches!(self.entry.source, Source::Video(_)) {
            let needs_new_image = cur_resized_img
                .as_ref()
                .map_or(true, |img| img.width() != width || img.height() != height);

            if needs_new_image {
                *cur_resized_img = Some(self.prepare_scaled_image(width, height)?);
            }
        }

        let mut video_frame = None;
        if matches!(self.entry.source, Source::Video(_)) {
            if let Some(animated_source) = self.animated_source.as_mut() {
                match &self.entry.scaling_mode {
                    ScalingMode::Stretch => animated_source.prepare(width, height),
                    ScalingMode::Fit(_) | ScalingMode::FitBlur | ScalingMode::Zoom => {
                        animated_source.prepare_unscaled()
                    }
                }
                .map_err(|e| DrawError::ImageDecode {
                    path: PathBuf::from("animated"),
                    reason: format!("Failed to prepare animated source: {}", e),
                })?;

                let frame = animated_source.next_frame()
                    .map_err(|e| DrawError::ImageDecode {
                        path: PathBuf::from("animated"),
                        reason: format!("Failed to get next frame: {}", e),
                    })?;
                video_frame = Some(frame);
            }
        }

        let video_frame = video_frame
            .map(|frame| {
                self.prepare_video_frame(frame.payload, frame.is_placeholder, width, height)
            })
            .transpose()?;

        // Now we can get mutable access to the layer
        let layer = self.layers.get_mut(layer_idx).ok_or(DrawError::NoSource)?;
        let pool = layer.pool.as_mut().ok_or(DrawError::NoSource)?;

        let buffer = if let Some(frame) = video_frame {
            match frame {
                PreparedVideoFrame::Bgrx {
                    data,
                    width,
                    height,
                    stride,
                } => crate::draw::canvas_from_bgrx(pool, &data, width, height, stride)?,
                PreparedVideoFrame::FitBlurBgrx {
                    background,
                    data,
                    frame_width,
                    frame_height,
                    frame_stride,
                    layer_width,
                    layer_height,
                } => crate::draw::canvas_from_fit_blur_bgrx_with_workspace(
                    pool,
                    &background,
                    &data,
                    frame_width,
                    frame_height,
                    frame_stride,
                    layer_width,
                    layer_height,
                    &mut self.fit_blur_bgrx_workspace,
                )?,
                PreparedVideoFrame::Image(image) => crate::draw::canvas(
                    pool,
                    &image,
                    width as i32,
                    height as i32,
                    width as i32 * 4,
                )?,
            }
        } else {
            let image = cur_resized_img.as_ref().expect("cur_resized_img was just set");
            crate::draw::canvas(pool, image, width as i32, height as i32, width as i32 * 4)?
        };

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

    fn prepare_scaled_image(&mut self, width: u32, height: u32) -> Result<Arc<DynamicImage>, DrawError> {
        // Clone to avoid borrow conflicts when calling methods that mutate self
        let source = self.current_source.clone().ok_or(DrawError::NoSource)?;

        match source {
            Source::Path(ref path) => self
                .scale_image_from_path(path, width, height)
                .map(Arc::new),
            Source::Color(Color::Single([r, g, b])) => {
                Ok(Arc::new(self.generate_solid_color([r, g, b], width, height)))
            }
            Source::Color(Color::Gradient(ref gradient)) => {
                self.generate_gradient(gradient, width, height).map(Arc::new)
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

                match frame.payload {
                    FramePayload::Image(image) => Ok(image),
                    FramePayload::Bgrx { .. } => Err(DrawError::ImageDecode {
                        path: PathBuf::from("animated"),
                        reason: "Unexpected raw frame payload for image path".to_string(),
                    }),
                }
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

        let img = self.current_image.as_ref().expect("current_image was set by the is_none check above");
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
        match &self.entry.scaling_mode {
            ScalingMode::Fit(color) => crate::scaler::fit(img, color, width, height),
            ScalingMode::FitBlur => crate::scaler::fit_blur(img, width, height),
            ScalingMode::Zoom => crate::scaler::zoom(img, width, height),
            ScalingMode::Stretch => crate::scaler::stretch(img, width, height),
        }
    }

    fn prepare_video_frame(
        &mut self,
        payload: FramePayload,
        is_placeholder: bool,
        width: u32,
        height: u32,
    ) -> Result<PreparedVideoFrame, DrawError> {
        match payload {
            FramePayload::Bgrx {
                data,
                width: frame_width,
                height: frame_height,
                stride,
            } => {
                if matches!(&self.entry.scaling_mode, ScalingMode::Stretch)
                    && frame_width == width
                    && frame_height == height
                    && stride >= width as usize * 4
                {
                    return Ok(PreparedVideoFrame::Bgrx {
                        data,
                        width: frame_width,
                        height: frame_height,
                        stride,
                    });
                }

                if matches!(&self.entry.scaling_mode, ScalingMode::FitBlur) {
                    let background = self.prepare_fit_blur_video_bgrx_frame(
                        &data,
                        frame_width,
                        frame_height,
                        stride,
                        is_placeholder,
                        width,
                        height,
                    )?;
                    return Ok(PreparedVideoFrame::FitBlurBgrx {
                        background,
                        data,
                        frame_width,
                        frame_height,
                        frame_stride: stride,
                        layer_width: width,
                        layer_height: height,
                    });
                }

                let image = bgrx_to_dynamic_image(&data, frame_width, frame_height, stride)?;
                Ok(PreparedVideoFrame::Image(self.apply_scaling_mode(
                    &image, width, height,
                )))
            }
            FramePayload::Image(image) => {
                if matches!(&self.entry.scaling_mode, ScalingMode::FitBlur) {
                    return Ok(PreparedVideoFrame::Image(
                        self.prepare_fit_blur_video_frame(&image, is_placeholder, width, height),
                    ));
                }

                Ok(PreparedVideoFrame::Image(self.apply_scaling_mode(
                    &image, width, height,
                )))
            }
        }
    }

    fn prepare_fit_blur_video_frame(
        &mut self,
        image: &DynamicImage,
        is_placeholder: bool,
        width: u32,
        height: u32,
    ) -> DynamicImage {
        prepare_fit_blur_video_frame_from_cache(
            &mut self.fit_blur_background_cache,
            image,
            is_placeholder,
            width,
            height,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_fit_blur_video_bgrx_frame(
        &mut self,
        data: &[u8],
        frame_width: u32,
        frame_height: u32,
        frame_stride: usize,
        is_placeholder: bool,
        width: u32,
        height: u32,
    ) -> Result<Arc<[u8]>, DrawError> {
        prepare_fit_blur_video_bgrx_background_from_cache(
            &mut self.fit_blur_background_cache,
            data,
            frame_width,
            frame_height,
            frame_stride,
            is_placeholder,
            width,
            height,
        )
    }

    fn generate_solid_color(&self, color: [f32; 3], width: u32, height: u32) -> DynamicImage {
        DynamicImage::from(crate::colored::single(color, width, height))
    }

    fn generate_gradient(
        &self,
        gradient: &cosmic_ext_bg_config::Gradient,
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

                match VideoSource::new(video_config.clone()) {
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

                match AnimatedSource::new(animated_config.clone()) {
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

    fn watch_source(&mut self, tx: calloop::channel::SyncSender<(String, notify::Event)>) {
        let Source::Path(ref source) = self.entry.source else {
            self._watcher = None;
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
            Err(why) => {
                tracing::error!(?why, output = self.entry.output, "failed to create file watcher");
                self._watcher = None;
                return;
            }
        };

        tracing::debug!(output = self.entry.output, "watching source");

        if let Ok(m) = fs::metadata(source) {
            if m.is_dir() {
                if let Err(why) = watcher.watch(source, RecursiveMode::Recursive) {
                    tracing::error!(?why, ?source, "failed to watch directory");
                }
            } else if m.is_file() {
                if let Err(why) = watcher.watch(source, RecursiveMode::NonRecursive) {
                    tracing::error!(?why, ?source, "failed to watch file");
                }
            }
        }

        self._watcher = Some(watcher);
    }

    fn setup_animation_timer(&mut self) {
        // Remove existing animation timer if present
        if let Some(token) = self.animation_timer_token.take() {
            self.loop_handle.remove(token);
        }

        // Video playback is paced by Wayland frame callbacks. Timer-driven
        // redraws can race the compositor cadence and cause visible jitter.
        if matches!(self.entry.source, Source::Video(_)) {
            return;
        }

        // Get frame duration from the animated source
        let frame_duration = self
            .animated_source
            .as_ref()
            .map(|source| source.frame_duration())
            .unwrap_or(crate::source::DEFAULT_FRAME_DURATION);

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
                        .unwrap_or(crate::source::DEFAULT_FRAME_DURATION);

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

                        // Skip rotation when there's only one image — it would
                        // re-decode and re-draw the same wallpaper.
                        if item.image_queue.len() <= 1 {
                            return TimeoutAction::ToDuration(Duration::from_secs(rotation_freq));
                        }

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
        self.fit_blur_background_cache.clear();
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

fn bgrx_to_dynamic_image(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<DynamicImage, DrawError> {
    let row_bytes = width as usize * 4;
    if stride < row_bytes || data.len() < stride * height as usize {
        return Err(DrawError::ImageDecode {
            path: PathBuf::from("video"),
            reason: "Invalid BGRx frame dimensions".to_string(),
        });
    }

    let mut rgba = vec![0; row_bytes * height as usize];
    for row in 0..height as usize {
        let src_start = row * stride;
        let src_row = &data[src_start..src_start + row_bytes];
        let dst_start = row * row_bytes;
        let dst_row = &mut rgba[dst_start..dst_start + row_bytes];

        for (src, dst) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
            let [b, g, r, _x] = src else { unreachable!() };
            dst.copy_from_slice(&[*r, *g, *b, 255]);
        }
    }

    image::ImageBuffer::from_raw(width, height, rgba)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| DrawError::ImageDecode {
            path: PathBuf::from("video"),
            reason: "Failed to build image from BGRx frame".to_string(),
        })
}

fn prepare_fit_blur_video_frame_from_cache(
    fit_blur_background_cache: &mut Vec<FitBlurBackgroundCache>,
    image: &DynamicImage,
    is_placeholder: bool,
    width: u32,
    height: u32,
) -> DynamicImage {
    if is_placeholder {
        let background = crate::scaler::fit_blur_background_fast(image, width, height);
        return crate::scaler::compose_fit_blur(image, &background, width, height);
    }

    let background = fit_blur_background_from_cache(
        fit_blur_background_cache,
        image,
        width,
        height,
    );
    let background = bgrx_to_rgba_image(background, width, height)
        .expect("cached fit-blur background dimensions are valid");
    crate::scaler::compose_fit_blur(image, &background, width, height)
}

#[allow(clippy::too_many_arguments)]
fn prepare_fit_blur_video_bgrx_background_from_cache(
    fit_blur_background_cache: &mut Vec<FitBlurBackgroundCache>,
    data: &[u8],
    frame_width: u32,
    frame_height: u32,
    frame_stride: usize,
    is_placeholder: bool,
    width: u32,
    height: u32,
) -> Result<Arc<[u8]>, DrawError> {
    let row_bytes = frame_width as usize * 4;
    if frame_stride < row_bytes || data.len() < frame_stride * frame_height as usize {
        return Err(DrawError::ImageDecode {
            path: PathBuf::from("video"),
            reason: "Invalid BGRx frame dimensions".to_string(),
        });
    }

    let frame_identity = (frame_width, frame_height);
    if is_placeholder {
        let image = bgrx_to_dynamic_image(data, frame_width, frame_height, frame_stride)?;
        return Ok(fit_blur_background_bgrx_from_image(&image, width, height));
    }

    if let Some(index) = fit_blur_background_cache.iter().position(|cache| {
        cache.layer_width == width
            && cache.layer_height == height
            && cache.frame_width == frame_identity.0
            && cache.frame_height == frame_identity.1
    }) {
        let cache = &mut fit_blur_background_cache[index];
        cache.frames_since_refresh += 1;
        if cache.frames_since_refresh >= FIT_BLUR_BACKGROUND_REFRESH_FRAMES {
            let image = bgrx_to_dynamic_image(data, frame_width, frame_height, frame_stride)?;
            cache.background = fit_blur_background_bgrx_from_image(&image, width, height);
            cache.frames_since_refresh = 0;
        }
        return Ok(Arc::clone(&fit_blur_background_cache[index].background));
    }

    let image = bgrx_to_dynamic_image(data, frame_width, frame_height, frame_stride)?;
    fit_blur_background_cache.push(FitBlurBackgroundCache {
        layer_width: width,
        layer_height: height,
        frame_width,
        frame_height,
        frames_since_refresh: 0,
        background: fit_blur_background_bgrx_from_image(&image, width, height),
    });

    Ok(Arc::clone(
        &fit_blur_background_cache
            .last()
            .expect("fit_blur_background_cache was just pushed")
            .background,
    ))
}

fn fit_blur_background_from_cache<'a>(
    fit_blur_background_cache: &'a mut Vec<FitBlurBackgroundCache>,
    image: &DynamicImage,
    width: u32,
    height: u32,
) -> &'a [u8] {
    let frame_width = image.width();
    let frame_height = image.height();

    if let Some(index) = fit_blur_background_cache.iter().position(|cache| {
        cache.layer_width == width
            && cache.layer_height == height
            && cache.frame_width == frame_width
            && cache.frame_height == frame_height
    }) {
        let cache = &mut fit_blur_background_cache[index];
        cache.frames_since_refresh += 1;
        if cache.frames_since_refresh >= FIT_BLUR_BACKGROUND_REFRESH_FRAMES {
            cache.background = fit_blur_background_bgrx_from_image(image, width, height);
            cache.frames_since_refresh = 0;
        }
        return &fit_blur_background_cache[index].background;
    }

    fit_blur_background_cache.push(FitBlurBackgroundCache {
        layer_width: width,
        layer_height: height,
        frame_width,
        frame_height,
        frames_since_refresh: 0,
        background: fit_blur_background_bgrx_from_image(image, width, height),
    });

    &fit_blur_background_cache
        .last()
        .expect("fit_blur_background_cache was just pushed")
        .background
}

fn fit_blur_background_bgrx_from_image(image: &DynamicImage, width: u32, height: u32) -> Arc<[u8]> {
    let background = crate::scaler::fit_blur_background_fast(image, width, height);
    crate::scaler::fit_blur_background_to_bgrx(&background)
}

fn bgrx_to_rgba_image(data: &[u8], width: u32, height: u32) -> Option<image::RgbaImage> {
    let row_bytes = width as usize * 4;
    if data.len() < row_bytes * height as usize {
        return None;
    }

    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for pixel in data[..row_bytes * height as usize].chunks_exact(4) {
        let [b, g, r, _x] = pixel else { unreachable!() };
        rgba.extend_from_slice(&[*r, *g, *b, 255]);
    }

    image::ImageBuffer::from_raw(width, height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn fit_blur_placeholder_frame_does_not_populate_background_cache() {
        let mut cache = Vec::new();
        let placeholder = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            320,
            180,
            Rgba([0, 0, 0, 255]),
        ));
        let real_frame = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            320,
            180,
            Rgba([255, 0, 0, 255]),
        ));

        let _ = prepare_fit_blur_video_frame_from_cache(
            &mut cache,
            &placeholder,
            true,
            800,
            600,
        );
        assert!(cache.is_empty());

        let _ = prepare_fit_blur_video_frame_from_cache(
            &mut cache,
            &real_frame,
            false,
            800,
            600,
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn fit_blur_real_frame_refreshes_cached_background_periodically() {
        let mut cache = Vec::new();
        let red_frame = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            320,
            180,
            Rgba([255, 0, 0, 255]),
        ));
        let blue_frame = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            320,
            180,
            Rgba([0, 0, 255, 255]),
        ));

        let first = prepare_fit_blur_video_frame_from_cache(
            &mut cache,
            &red_frame,
            false,
            800,
            600,
        )
        .to_rgba8();
        assert_eq!(first.get_pixel(400, 10).0, [255, 0, 0, 255]);

        let stale_background = prepare_fit_blur_video_frame_from_cache(
            &mut cache,
            &blue_frame,
            false,
            800,
            600,
        )
        .to_rgba8();
        assert_eq!(stale_background.get_pixel(400, 10).0, [255, 0, 0, 255]);
        assert_eq!(stale_background.get_pixel(400, 300).0, [0, 0, 255, 255]);

        let mut refreshed_background = stale_background;
        for _ in 1..FIT_BLUR_BACKGROUND_REFRESH_FRAMES {
            refreshed_background = prepare_fit_blur_video_frame_from_cache(
                &mut cache,
                &blue_frame,
                false,
                800,
                600,
            )
            .to_rgba8();
        }

        assert_eq!(refreshed_background.get_pixel(400, 10).0, [0, 0, 255, 255]);
        assert_eq!(cache.len(), 1);
    }
}

/// Decodes JPEG XL image files into `image::DynamicImage` via `jxl-oxide`.
fn decode_jpegxl(path: &std::path::Path) -> eyre::Result<DynamicImage> {
    let file = File::open(path).map_err(|why| eyre!("failed to open jxl image file: {why}"))?;

    let decoder =
        JxlDecoder::new(file).map_err(|why| eyre!("failed to read jxl image header: {why}"))?;

    image::DynamicImage::from_decoder(decoder)
        .map_err(|why| eyre!("failed to decode jxl image: {why}"))
}
