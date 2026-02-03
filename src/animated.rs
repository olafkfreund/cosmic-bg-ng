// SPDX-License-Identifier: MPL-2.0

//! Animated image support for GIF, APNG, and animated WebP wallpapers.
//!
//! This module provides frame-by-frame playback of animated images,
//! respecting per-frame delay timings for smooth animation.

use crate::source::{Frame, SourceError, WallpaperSource};
use image::{codecs::gif::GifDecoder, AnimationDecoder, DynamicImage};
use std::{
    collections::VecDeque,
    fs::File,
    io::BufReader,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Helper to create decode errors with context
fn decode_error(context: &str, err: impl std::fmt::Display) -> SourceError {
    SourceError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{}: {}", context, err),
    ))
}

/// Helper to calculate frame delay from image::Delay
fn calculate_frame_delay(delay: image::Delay) -> Duration {
    let (numerator, denominator) = delay.numer_denom_ms();
    let delay_ms = ((numerator as f64 / denominator as f64) as u64).max(10);
    Duration::from_millis(delay_ms)
}

/// Configuration for animated image wallpapers
#[derive(Debug, Clone)]
pub struct AnimatedConfig {
    /// Path to the animated image file
    pub path: PathBuf,
    /// Optional FPS limit to reduce CPU usage
    pub fps_limit: Option<u32>,
    /// Number of times to loop (None = infinite)
    pub loop_count: Option<u32>,
}

/// A single frame from an animated image
#[derive(Debug, Clone)]
struct AnimatedFrame {
    /// The frame image data
    image: DynamicImage,
    /// Delay before showing the next frame
    delay: Duration,
}

/// Animated image wallpaper source
#[derive(Debug)]
pub struct AnimatedSource {
    config: AnimatedConfig,
    frames: VecDeque<AnimatedFrame>,
    current_frame_idx: usize,
    last_frame_time: Instant,
    current_frame_delay: Duration,
    loops_completed: u32,
    is_prepared: bool,
    target_size: Option<(u32, u32)>,
}

impl AnimatedSource {
    /// Create a new animated image source from configuration
    pub fn new(config: AnimatedConfig) -> Result<Self, SourceError> {
        Ok(Self {
            config,
            frames: VecDeque::new(),
            current_frame_idx: 0,
            last_frame_time: Instant::now(),
            current_frame_delay: Duration::from_millis(100),
            loops_completed: 0,
            is_prepared: false,
            target_size: None,
        })
    }

    /// Load frames from the animated image file
    fn load_frames(&mut self) -> Result<(), SourceError> {
        let path = &self.config.path;

        // Detect format from extension
        let format = Self::detect_format(path)?;

        match format {
            AnimatedFormat::Gif => self.load_gif_frames()?,
            AnimatedFormat::Apng => self.load_apng_frames()?,
            AnimatedFormat::WebP => self.load_webp_frames()?,
        }

        if self.frames.is_empty() {
            return Err(SourceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No frames found in animated image",
            )));
        }

        // Set initial frame delay
        if let Some(frame) = self.frames.front() {
            self.current_frame_delay = self.apply_fps_limit(frame.delay);
        }

        tracing::info!(
            path = ?self.config.path,
            frame_count = self.frames.len(),
            "Loaded animated image"
        );

        Ok(())
    }

    /// Detect the animated image format from file extension
    fn detect_format(path: &PathBuf) -> Result<AnimatedFormat, SourceError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .ok_or_else(|| decode_error("No file extension", ""))?;

        match ext.as_str() {
            "gif" => Ok(AnimatedFormat::Gif),
            "apng" | "png" => Ok(AnimatedFormat::Apng),
            "webp" => Ok(AnimatedFormat::WebP),
            _ => Err(decode_error(
                "Unsupported animated format",
                ext,
            )),
        }
    }

    /// Load frames from a GIF file
    fn load_gif_frames(&mut self) -> Result<(), SourceError> {
        let file = File::open(&self.config.path)?;
        let reader = BufReader::new(file);

        let decoder = GifDecoder::new(reader)
            .map_err(|e| decode_error("Failed to decode GIF", e))?;

        for frame_result in decoder.into_frames() {
            let frame = frame_result
                .map_err(|e| decode_error("Failed to decode GIF frame", e))?;

            let delay = calculate_frame_delay(frame.delay());
            let image = DynamicImage::ImageRgba8(frame.into_buffer());

            self.frames.push_back(AnimatedFrame { image, delay });
        }

        Ok(())
    }

    /// Load frames from an APNG file
    fn load_apng_frames(&mut self) -> Result<(), SourceError> {
        use image::codecs::png::PngDecoder;

        let file = File::open(&self.config.path)?;
        let reader = BufReader::new(file);

        let decoder = PngDecoder::new(reader)
            .map_err(|e| decode_error("Failed to decode PNG", e))?;

        // Check if it's actually animated
        if !decoder.is_apng().unwrap_or(false) {
            // Static PNG, load as single frame
            let image = image::open(&self.config.path)
                .map_err(|e| decode_error("Failed to decode image", e))?;

            self.frames.push_back(AnimatedFrame {
                image,
                delay: Duration::MAX, // Static image
            });

            return Ok(());
        }

        // Load APNG frames
        let apng_decoder = decoder.apng()
            .map_err(|e| decode_error("Failed to create APNG decoder", e))?;

        for frame_result in apng_decoder.into_frames() {
            let frame = frame_result
                .map_err(|e| decode_error("Failed to decode APNG frame", e))?;

            let delay = calculate_frame_delay(frame.delay());
            let image = DynamicImage::ImageRgba8(frame.into_buffer());

            self.frames.push_back(AnimatedFrame { image, delay });
        }

        Ok(())
    }

    /// Load frames from an animated WebP file
    fn load_webp_frames(&mut self) -> Result<(), SourceError> {
        use image::codecs::webp::WebPDecoder;

        let file = File::open(&self.config.path)?;
        let reader = BufReader::new(file);

        let decoder = WebPDecoder::new(reader)
            .map_err(|e| decode_error("Failed to decode WebP", e))?;

        // Check if it has animation
        if !decoder.has_animation() {
            // Static WebP, load as single frame
            let image = image::open(&self.config.path)
                .map_err(|e| decode_error("Failed to decode image", e))?;

            self.frames.push_back(AnimatedFrame {
                image,
                delay: Duration::MAX,
            });

            return Ok(());
        }

        // Load WebP frames
        for frame_result in decoder.into_frames() {
            let frame = frame_result
                .map_err(|e| decode_error("Failed to decode WebP frame", e))?;

            let delay = calculate_frame_delay(frame.delay());
            let image = DynamicImage::ImageRgba8(frame.into_buffer());

            self.frames.push_back(AnimatedFrame { image, delay });
        }

        Ok(())
    }

    /// Apply FPS limit to frame delay if configured
    fn apply_fps_limit(&self, delay: Duration) -> Duration {
        if let Some(fps_limit) = self.config.fps_limit {
            let min_delay = Duration::from_secs_f64(1.0 / fps_limit as f64);
            delay.max(min_delay)
        } else {
            delay
        }
    }

    /// Advance to the next frame
    fn advance_frame(&mut self) -> bool {
        if self.frames.len() <= 1 {
            return false; // Static image or empty
        }

        self.current_frame_idx = (self.current_frame_idx + 1) % self.frames.len();

        // Check for loop completion
        if self.current_frame_idx == 0 {
            self.loops_completed += 1;

            // Check if we've hit the loop limit
            if let Some(max_loops) = self.config.loop_count {
                if self.loops_completed >= max_loops {
                    return false; // Stop animating
                }
            }
        }

        // Update delay for next frame
        if let Some(frame) = self.frames.get(self.current_frame_idx) {
            self.current_frame_delay = self.apply_fps_limit(frame.delay);
        }

        self.last_frame_time = Instant::now();

        true
    }
}

impl WallpaperSource for AnimatedSource {
    fn next_frame(&mut self) -> Result<Frame, SourceError> {
        if !self.is_prepared {
            return Err(decode_error("Animated source not prepared", ""));
        }

        // Check if it's time to advance
        let elapsed = self.last_frame_time.elapsed();
        if elapsed >= self.current_frame_delay {
            self.advance_frame();
        }

        // Get current frame
        let frame = self
            .frames
            .get(self.current_frame_idx)
            .ok_or_else(|| decode_error("No frames available", ""))?;

        Ok(Frame {
            image: frame.image.clone(),
            timestamp: Instant::now(),
        })
    }

    fn frame_duration(&self) -> Duration {
        self.current_frame_delay
    }

    fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        self.target_size = Some((width, height));

        if self.frames.is_empty() {
            self.load_frames()?;
        }

        self.last_frame_time = Instant::now();
        self.is_prepared = true;

        Ok(())
    }

    fn release(&mut self) {
        self.frames.clear();
        self.current_frame_idx = 0;
        self.loops_completed = 0;
        self.is_prepared = false;

        tracing::debug!("Animated source released");
    }

    fn description(&self) -> String {
        let format = Self::detect_format(&self.config.path)
            .map(|f| format!("{:?}", f))
            .unwrap_or_else(|_| "Unknown".to_string());

        format!(
            "Animated {}: {} ({} frames)",
            format,
            self.config.path.display(),
            self.frames.len()
        )
    }
}

impl Drop for AnimatedSource {
    fn drop(&mut self) {
        self.release();
    }
}

/// Supported animated image formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimatedFormat {
    Gif,
    Apng,
    WebP,
}

/// Check if a file is an animated image
///
/// Utility function for detecting animated formats. Currently unused but provides
/// format detection capability for future animation support in wallpaper loading.
#[allow(dead_code)]
pub fn is_animated_image(path: &PathBuf) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    matches!(ext.as_deref(), Some("gif" | "apng" | "webp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_animated_config() {
        let config = AnimatedConfig {
            path: PathBuf::from("/tmp/test.gif"),
            fps_limit: Some(30),
            loop_count: None,
        };

        let source = AnimatedSource::new(config);
        assert!(source.is_ok());

        let source = source.unwrap();
        assert!(!source.is_prepared);
    }

    #[test]
    fn test_format_detection() {
        assert_eq!(
            AnimatedSource::detect_format(&PathBuf::from("test.gif")).unwrap(),
            AnimatedFormat::Gif
        );
        assert_eq!(
            AnimatedSource::detect_format(&PathBuf::from("test.apng")).unwrap(),
            AnimatedFormat::Apng
        );
        assert_eq!(
            AnimatedSource::detect_format(&PathBuf::from("test.webp")).unwrap(),
            AnimatedFormat::WebP
        );
    }

    #[test]
    fn test_is_animated_image() {
        assert!(is_animated_image(&PathBuf::from("test.gif")));
        assert!(is_animated_image(&PathBuf::from("test.webp")));
        assert!(!is_animated_image(&PathBuf::from("test.jpg")));
        assert!(!is_animated_image(&PathBuf::from("test.png")));
    }

    #[test]
    fn test_fps_limit() {
        let config = AnimatedConfig {
            path: PathBuf::from("/tmp/test.gif"),
            fps_limit: Some(30),
            loop_count: None,
        };

        let source = AnimatedSource::new(config).unwrap();

        // 30fps = ~33ms per frame minimum
        let limited = source.apply_fps_limit(Duration::from_millis(10));
        assert!(limited >= Duration::from_millis(33));

        // Slower frames should not be affected
        let slow = source.apply_fps_limit(Duration::from_millis(100));
        assert_eq!(slow, Duration::from_millis(100));
    }
}
