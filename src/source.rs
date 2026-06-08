// SPDX-License-Identifier: MPL-2.0

//! Extensible wallpaper source trait and implementations.
//!
//! This module defines the WallpaperSource trait for pluggable background sources
//! (static images, colors, animations, videos). Most types are not yet integrated
//! but provide API for future extensibility.

#![allow(dead_code)]

use cosmic_ext_bg_config::Color;
use image::DynamicImage;
use std::{
    fs::File,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;

/// Default frame duration (~30 fps) used when no source-specific timing is available.
pub const DEFAULT_FRAME_DURATION: Duration = Duration::from_millis(33);

/// Fallback resolution when no output dimensions have been configured yet.
pub const FALLBACK_RESOLUTION: (u32, u32) = (1920, 1080);

/// A single frame to be rendered
#[derive(Debug)]
pub struct Frame {
    pub payload: FramePayload,
    pub timestamp: Instant,
    pub is_placeholder: bool,
}

#[derive(Debug, Clone)]
pub enum FramePayload {
    Image(Arc<DynamicImage>),
    Bgrx {
        data: Arc<[u8]>,
        width: u32,
        height: u32,
        stride: usize,
    },
}

impl Frame {
    pub fn from_image(image: Arc<DynamicImage>) -> Self {
        Self {
            payload: FramePayload::Image(image),
            timestamp: Instant::now(),
            is_placeholder: false,
        }
    }

    pub fn placeholder(payload: FramePayload) -> Self {
        Self {
            payload,
            timestamp: Instant::now(),
            is_placeholder: true,
        }
    }
}

/// Errors that can occur when working with wallpaper sources
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("Failed to decode image: {0}")]
    Decode(#[from] image::ImageError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Gradient error: {0}")]
    Gradient(String),
    #[error("JPEG XL decode error: {0}")]
    JpegXl(#[from] eyre::Report),
}

impl SourceError {
    /// Create an I/O-based source error with the given kind and message.
    pub fn io(kind: std::io::ErrorKind, msg: impl Into<String>) -> Self {
        Self::Io(std::io::Error::new(kind, msg.into()))
    }
}

/// Trait for extensible wallpaper source types
pub trait WallpaperSource: Send + Sync {
    /// Get the next frame to render
    fn next_frame(&mut self) -> Result<Frame, SourceError>;

    /// Duration until next frame should be rendered
    /// Returns Duration::MAX for static sources
    fn frame_duration(&self) -> Duration;

    /// Whether this source requires continuous rendering
    fn is_animated(&self) -> bool;

    /// Prepare source for rendering at given dimensions
    /// This is called when the output size changes
    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError>;

    /// Prepare source without forcing decoded frames to the output dimensions.
    fn prepare_unscaled(&mut self) -> Result<(), SourceError> {
        self.prepare(FALLBACK_RESOLUTION.0, FALLBACK_RESOLUTION.1)
    }

    /// Release resources when no longer needed
    fn release(&mut self);

    /// Get a description of this source for debugging
    fn description(&self) -> String;
}

/// Static image source for single image files
#[derive(Debug)]
pub struct StaticSource {
    path: PathBuf,
    cached_image: Option<DynamicImage>,
    prepared_size: Option<(u32, u32)>,
}

impl StaticSource {
    /// Create a new static image source from a file path
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            cached_image: None,
            prepared_size: None,
        }
    }

    /// Decode the image from the file path
    fn load_image(&self) -> Result<DynamicImage, SourceError> {
        // Handle JPEG XL format specially
        if let Some(ext) = self.path.extension() {
            if ext == "jxl" {
                return decode_jpegxl(&self.path);
            }
        }

        // Use standard image decoder for other formats
        let reader = image::ImageReader::open(&self.path)?;
        let image = reader.with_guessed_format()?.decode()?;
        Ok(image)
    }
}

impl WallpaperSource for StaticSource {
    fn next_frame(&mut self) -> Result<Frame, SourceError> {
        if self.cached_image.is_none() {
            self.cached_image = Some(self.load_image()?);
        }

        Ok(Frame {
            payload: FramePayload::Image(Arc::new(self.cached_image.as_ref().unwrap().clone())),
            timestamp: Instant::now(),
            is_placeholder: false,
        })
    }

    fn frame_duration(&self) -> Duration {
        Duration::MAX // Static images don't need updates
    }

    fn is_animated(&self) -> bool {
        false
    }

    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        self.prepared_size = Some((width, height));
        // Pre-load the image if not already cached
        if self.cached_image.is_none() {
            self.cached_image = Some(self.load_image()?);
        }
        Ok(())
    }

    fn release(&mut self) {
        self.cached_image = None;
        self.prepared_size = None;
    }

    fn description(&self) -> String {
        format!("Static image: {}", self.path.display())
    }
}

/// Color source for solid colors and gradients
#[derive(Debug)]
pub struct ColorSource {
    color: Color,
    generated: Option<DynamicImage>,
    size: Option<(u32, u32)>,
}

impl ColorSource {
    /// Create a new color source
    pub fn new(color: Color) -> Self {
        Self {
            color,
            generated: None,
            size: None,
        }
    }

    /// Generate the color image
    fn generate_image(&self, width: u32, height: u32) -> Result<DynamicImage, SourceError> {
        match &self.color {
            Color::Single([r, g, b]) => {
                let buffer = crate::colored::single([*r, *g, *b], width, height);
                Ok(DynamicImage::from(buffer))
            }
            Color::Gradient(gradient) => {
                let buffer = crate::colored::gradient(gradient, width, height)
                    .map_err(|e| SourceError::Gradient(e.to_string()))?;
                Ok(DynamicImage::from(buffer))
            }
        }
    }
}

impl WallpaperSource for ColorSource {
    fn next_frame(&mut self) -> Result<Frame, SourceError> {
        let (width, height) = self.size.unwrap_or(FALLBACK_RESOLUTION);

        if self.generated.is_none() {
            self.generated = Some(self.generate_image(width, height)?);
        }

        Ok(Frame {
            payload: FramePayload::Image(Arc::new(self.generated.as_ref().unwrap().clone())),
            timestamp: Instant::now(),
            is_placeholder: false,
        })
    }

    fn frame_duration(&self) -> Duration {
        Duration::MAX // Colors don't animate
    }

    fn is_animated(&self) -> bool {
        false
    }

    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        self.size = Some((width, height));
        // Regenerate if size changed
        if self.generated.is_some() {
            self.generated = Some(self.generate_image(width, height)?);
        }
        Ok(())
    }

    fn release(&mut self) {
        self.generated = None;
    }

    fn description(&self) -> String {
        match &self.color {
            Color::Single([r, g, b]) => format!("Solid color: RGB({}, {}, {})", r, g, b),
            Color::Gradient(g) => format!("Gradient: {} colors at {} degrees", g.colors.len(), g.radius),
        }
    }
}

/// Decode JPEG XL image files into `image::DynamicImage` via `jxl-oxide`.
fn decode_jpegxl(path: &std::path::Path) -> Result<DynamicImage, SourceError> {
    use eyre::eyre;
    use jxl_oxide::integration::JxlDecoder;

    let file = File::open(path)
        .map_err(|why| eyre!("failed to open jxl image file: {why}"))?;

    let decoder = JxlDecoder::new(file)
        .map_err(|why| eyre!("failed to read jxl image header: {why}"))?;

    let image = DynamicImage::from_decoder(decoder)
        .map_err(|why| eyre!("failed to decode jxl image: {why}"))?;

    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_source_single() {
        let mut source = ColorSource::new(Color::Single([1.0, 0.0, 0.0]));
        source.prepare(100, 100).unwrap();
        let frame = source.next_frame().unwrap();
        let FramePayload::Image(image) = frame.payload else {
            panic!("expected image payload");
        };
        assert_eq!(image.width(), 100);
        assert_eq!(image.height(), 100);
        assert!(!source.is_animated());
    }

    #[test]
    fn test_source_description() {
        let color_source = ColorSource::new(Color::Single([1.0, 0.5, 0.0]));
        assert!(color_source.description().contains("Solid color"));
    }
}
