// SPDX-License-Identifier: MPL-2.0

use crate::source::{Frame, SourceError, WallpaperSource};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use image::{DynamicImage, ImageBuffer, Rgba};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Helper to convert GStreamer errors to SourceError
fn gst_error(message: impl Into<String>) -> SourceError {
    SourceError::Io(std::io::Error::new(std::io::ErrorKind::Other, message.into()))
}

/// Helper to create GStreamer elements
fn create_element(name: &str) -> Result<gst::Element, SourceError> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|e| gst_error(format!("Failed to create {}: {}", name, e)))
}

/// Helper to link multiple GStreamer elements
fn link_elements(elements: &[&gst::Element]) -> Result<(), SourceError> {
    gst::Element::link_many(elements)
        .map_err(|e| gst_error(format!("Failed to link elements: {}", e)))
}

/// Video playback configuration
///
/// Note: Audio is not supported for desktop wallpapers - only video frames are rendered.
#[derive(Debug, Clone)]
pub struct VideoConfig {
    /// Path to the video file
    pub path: PathBuf,
    /// Whether to loop playback
    pub loop_playback: bool,
    /// Playback speed multiplier (1.0 = normal)
    pub playback_speed: f64,
    /// Whether to use hardware acceleration
    pub hw_accel: bool,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            loop_playback: true,
            playback_speed: 1.0,
            hw_accel: true,
        }
    }
}

/// Video wallpaper source with GStreamer backend
#[derive(Debug)]
pub struct VideoSource {
    config: VideoConfig,
    pipeline: Option<gst::Pipeline>,
    appsink: Option<gst_app::AppSink>,
    current_frame: Arc<Mutex<Option<DynamicImage>>>,
    target_size: Option<(u32, u32)>,
    frame_duration: Duration,
    is_playing: bool,
    is_prepared: bool,
}

impl VideoSource {
    /// Create a new video source from a configuration
    pub fn new(config: VideoConfig) -> Result<Self, SourceError> {
        // Initialize GStreamer if not already initialized
        gst::init().map_err(|e| gst_error(format!("GStreamer initialization failed: {}", e)))?;

        Ok(Self {
            config,
            pipeline: None,
            appsink: None,
            current_frame: Arc::new(Mutex::new(None)),
            target_size: None,
            frame_duration: Duration::from_millis(33), // Default to ~30fps
            is_playing: false,
            is_prepared: false,
        })
    }

    /// Build the GStreamer pipeline for video playback
    fn build_pipeline(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        let path = self.config.path.to_str().ok_or_else(|| gst_error("Invalid video path"))?;

        // Detect hardware acceleration capabilities
        let hw_decode = if self.config.hw_accel {
            Self::detect_hw_decoder()
        } else {
            None
        };

        // Build pipeline string with hardware acceleration if available
        let decode_element = match hw_decode {
            Some(HwDecoder::VaApi) => "vaapidecodebin",
            Some(HwDecoder::Nvdec) => "nvdec",
            None => "decodebin",
        };

        tracing::info!(
            hw_accel = ?hw_decode,
            decoder = decode_element,
            "Building video pipeline"
        );

        // Create pipeline elements
        let pipeline = gst::Pipeline::new();

        let filesrc = gst::ElementFactory::make("filesrc")
            .property("location", path)
            .build()
            .map_err(|e| gst_error(format!("Failed to create filesrc: {}", e)))?;

        let decodebin = create_element(decode_element)?;
        let videoconvert = create_element("videoconvert")?;
        let videoscale = create_element("videoscale")?;

        let appsink = gst_app::AppSink::builder()
            .name("sink")
            .build();

        // Configure appsink caps for RGBA format
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", width as i32)
            .field("height", height as i32)
            .build();

        appsink.set_caps(Some(&caps));
        appsink.set_property("emit-signals", true);
        appsink.set_property("sync", false); // Don't sync to clock for wallpapers

        // Add elements to pipeline
        pipeline
            .add_many([&filesrc, &decodebin, &videoconvert, &videoscale, appsink.upcast_ref()])
            .map_err(|e| gst_error(format!("Failed to add elements to pipeline: {}", e)))?;

        // Link static elements
        link_elements(&[&filesrc, &decodebin])?;
        link_elements(&[&videoconvert, &videoscale, appsink.upcast_ref()])?;

        // Handle dynamic pad linking from decodebin
        let videoconvert_weak = videoconvert.downgrade();
        decodebin.connect_pad_added(move |_src, src_pad| {
            let Some(videoconvert) = videoconvert_weak.upgrade() else {
                return;
            };

            let sink_pad = videoconvert
                .static_pad("sink")
                .expect("videoconvert has no sink pad");

            if sink_pad.is_linked() {
                return;
            }

            let caps = src_pad.current_caps().unwrap();
            let structure = caps.structure(0).unwrap();
            let name = structure.name();

            if name.starts_with("video/") {
                if let Err(e) = src_pad.link(&sink_pad) {
                    tracing::error!("Failed to link decodebin pad: {}", e);
                }
            }
        });

        // Setup appsink callbacks
        let current_frame = Arc::clone(&self.current_frame);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;

                    if let Some(buffer) = sample.buffer() {
                        let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                        let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                        let structure = caps.structure(0).ok_or(gst::FlowError::Error)?;

                        let width = structure
                            .get::<i32>("width")
                            .map_err(|_| gst::FlowError::Error)? as u32;
                        let height = structure
                            .get::<i32>("height")
                            .map_err(|_| gst::FlowError::Error)? as u32;

                        if let Some(img_buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(
                            width,
                            height,
                            map.as_slice().to_vec(),
                        ) {
                            let image = DynamicImage::ImageRgba8(img_buffer);
                            *current_frame.lock().unwrap() = Some(image);
                        }
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Store pipeline and appsink
        self.pipeline = Some(pipeline.clone());
        self.appsink = Some(appsink);

        // Set playback speed
        if (self.config.playback_speed - 1.0).abs() > f64::EPSILON {
            pipeline
                .seek(
                    self.config.playback_speed,
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    gst::SeekType::Set,
                    gst::ClockTime::from_seconds(0),
                    gst::SeekType::None,
                    gst::ClockTime::NONE,
                )
                .ok();
        }

        Ok(())
    }

    /// Detect available hardware decoder
    fn detect_hw_decoder() -> Option<HwDecoder> {
        // Check for VA-API support (Intel, AMD)
        if gst::ElementFactory::find("vaapidecodebin").is_some() {
            tracing::info!("VA-API hardware acceleration available");
            return Some(HwDecoder::VaApi);
        }

        // Check for NVDEC support (NVIDIA)
        if gst::ElementFactory::find("nvdec").is_some() {
            tracing::info!("NVDEC hardware acceleration available");
            return Some(HwDecoder::Nvdec);
        }

        tracing::info!("No hardware acceleration available, using software decode");
        None
    }

    /// Start video playback
    fn play(&mut self) -> Result<(), SourceError> {
        if let Some(ref pipeline) = self.pipeline {
            pipeline
                .set_state(gst::State::Playing)
                .map_err(|e| gst_error(format!("Failed to start playback: {}", e)))?;
            self.is_playing = true;
            tracing::debug!("Video playback started");
        }
        Ok(())
    }

    /// Pause video playback
    fn pause(&mut self) -> Result<(), SourceError> {
        if let Some(ref pipeline) = self.pipeline {
            pipeline
                .set_state(gst::State::Paused)
                .map_err(|e| gst_error(format!("Failed to pause playback: {}", e)))?;
            self.is_playing = false;
            tracing::debug!("Video playback paused");
        }
        Ok(())
    }

    /// Check if video has reached end and loop if configured
    fn check_eos(&mut self) -> Result<(), SourceError> {
        if !self.config.loop_playback {
            return Ok(());
        }

        let Some(ref pipeline) = self.pipeline else {
            return Ok(());
        };

        let Some(bus) = pipeline.bus() else {
            return Err(gst_error("No bus available"));
        };

        // Check for EOS message (non-blocking)
        let Some(msg) = bus.pop_filtered(&[gst::MessageType::Eos]) else {
            return Ok(());
        };

        if let gst::MessageView::Eos(_) = msg.view() {
            tracing::debug!("Video reached end, looping");
            // Seek back to start
            pipeline
                .seek_simple(
                    gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                    gst::ClockTime::from_seconds(0),
                )
                .ok();
        }

        Ok(())
    }
}

impl WallpaperSource for VideoSource {
    fn next_frame(&mut self) -> Result<Frame, SourceError> {
        if !self.is_prepared {
            return Err(gst_error("Video source not prepared"));
        }

        if !self.is_playing {
            self.play()?;
        }

        // Check for end-of-stream and loop if needed
        self.check_eos()?;

        // Get current frame from buffer
        let frame_opt = self.current_frame.lock().unwrap().clone();

        if let Some(image) = frame_opt {
            Ok(Frame {
                image,
                timestamp: Instant::now(),
            })
        } else {
            // No frame yet, return black frame
            let (width, height) = self.target_size.unwrap_or((1920, 1080));
            let black = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 255]));
            Ok(Frame {
                image: DynamicImage::ImageRgba8(black),
                timestamp: Instant::now(),
            })
        }
    }

    fn frame_duration(&self) -> Duration {
        self.frame_duration
    }

    fn is_animated(&self) -> bool {
        true
    }

    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        self.target_size = Some((width, height));

        // Build pipeline if not already built
        if self.pipeline.is_none() {
            self.build_pipeline(width, height)?;
        }

        self.is_prepared = true;
        Ok(())
    }

    fn release(&mut self) {
        // Stop playback and cleanup
        if let Some(ref pipeline) = self.pipeline {
            let _ = pipeline.set_state(gst::State::Null);
        }

        self.pipeline = None;
        self.appsink = None;
        self.current_frame = Arc::new(Mutex::new(None));
        self.is_playing = false;
        self.is_prepared = false;

        tracing::debug!("Video source released");
    }

    fn description(&self) -> String {
        format!(
            "Video: {} (loop: {}, hw_accel: {})",
            self.config.path.display(),
            self.config.loop_playback,
            self.config.hw_accel
        )
    }
}

/// Hardware decoder types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HwDecoder {
    VaApi,
    Nvdec,
}

impl Drop for VideoSource {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_config_defaults() {
        let config = VideoConfig::default();
        assert!(config.loop_playback);
        assert_eq!(config.playback_speed, 1.0);
        assert!(config.hw_accel);
    }

    #[test]
    fn test_video_source_creation() {
        let config = VideoConfig {
            path: PathBuf::from("/tmp/test.mp4"),
            ..Default::default()
        };

        let result = VideoSource::new(config);
        assert!(result.is_ok());

        let source = result.unwrap();
        assert!(!source.is_playing);
        assert!(!source.is_prepared);
        assert!(source.is_animated());
    }

    #[test]
    fn test_video_source_description() {
        let config = VideoConfig {
            path: PathBuf::from("/tmp/video.mp4"),
            loop_playback: true,
            hw_accel: true,
            ..Default::default()
        };

        let source = VideoSource::new(config).unwrap();
        let desc = source.description();

        assert!(desc.contains("Video:"));
        assert!(desc.contains("/tmp/video.mp4"));
        assert!(desc.contains("loop: true"));
    }
}
