// SPDX-License-Identifier: MPL-2.0

//! Extensible wallpaper source trait and implementations.
//!
//! This module defines the WallpaperSource trait for pluggable background sources
//! (static images, colors, animations, videos). Most types are not yet integrated
//! but provide API for future extensibility.

#![allow(dead_code)]

use cosmic_bg_config::Color;
use image::DynamicImage;
use std::{
    fs::File,
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;

/// A single frame to be rendered
#[derive(Debug)]
pub struct Frame {
    pub image: DynamicImage,
    pub timestamp: Instant,
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
            image: self.cached_image.as_ref().unwrap().clone(),
            timestamp: Instant::now(),
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
        let (width, height) = self.size.unwrap_or((1920, 1080));

        if self.generated.is_none() {
            self.generated = Some(self.generate_image(width, height)?);
        }

        Ok(Frame {
            image: self.generated.as_ref().unwrap().clone(),
            timestamp: Instant::now(),
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
        assert_eq!(frame.image.width(), 100);
        assert_eq!(frame.image.height(), 100);
        assert!(!source.is_animated());
    }

    #[test]
    fn test_source_description() {
        let color_source = ColorSource::new(Color::Single([1.0, 0.5, 0.0]));
        assert!(color_source.description().contains("Solid color"));
    }
}
