// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

/// Errors that can occur during wallpaper operations.
///
/// This enum provides comprehensive error variants for all wallpaper operations.
/// Some variants are not currently used but are retained for complete API coverage
/// and future functionality.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum WallpaperError {
    /// Failed to bind a Wayland protocol.
    #[error("Failed to bind Wayland protocol {protocol}: {details}")]
    WaylandProtocol {
        protocol: &'static str,
        details: String,
    },

    /// Failed to decode an image file.
    #[error("Image decode failed: {path}")]
    ImageDecode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },

    /// Failed to decode a JPEG XL image.
    #[error("JPEG XL decode failed: {path}")]
    JxlDecode {
        path: PathBuf,
        #[source]
        source: eyre::Report,
    },

    /// Configuration system error.
    #[error("Configuration error: {0}")]
    Config(#[from] cosmic_config::Error),

    /// Failed to create event loop source.
    #[error("Failed to create event loop source: {details}")]
    EventLoopSource { details: String },

    /// Failed to insert source into event loop.
    #[error("Failed to insert {source_type} into event loop")]
    EventLoopInsert { source_type: &'static str },

    /// Missing required data for rendering.
    #[error("Missing required data: {what}")]
    MissingData { what: &'static str },

    /// Buffer pool operation failed.
    #[error("Buffer pool operation failed: {operation}")]
    BufferPool {
        operation: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Gradient generation error.
    #[error("Gradient error: {0}")]
    Gradient(String),
}

/// Result type alias for wallpaper operations.
///
/// Provided for convenience but currently unused. Available for future error handling
/// consistency across the codebase.
#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, WallpaperError>;
