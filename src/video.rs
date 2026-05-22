// SPDX-License-Identifier: MPL-2.0

use crate::source::{Frame, FramePayload, SourceError, WallpaperSource};
use cosmic_ext_bg_config::VideoConfig;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use image::{DynamicImage, ImageBuffer, Rgba};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const VIDEO_TARGET_FPS: u32 = 60;
const VIDEO_FRAME_DURATION: Duration = Duration::from_micros(1_000_000 / VIDEO_TARGET_FPS as u64);

/// Helper to convert GStreamer errors to SourceError
fn gst_error(message: impl Into<String>) -> SourceError {
    SourceError::io(std::io::ErrorKind::Other, message)
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

/// Video wallpaper source with GStreamer backend
#[derive(Debug)]
pub struct VideoSource {
    config: VideoConfig,
    pipeline: Option<gst::Pipeline>,
    appsink: Option<gst_app::AppSink>,
    last_frame: Option<FramePayload>,
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
            last_frame: None,
            target_size: None,
            frame_duration: VIDEO_FRAME_DURATION,
            is_playing: false,
            is_prepared: false,
        })
    }

    /// Build the GStreamer pipeline for video playback
    fn build_pipeline(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        let path = self.config.path.to_str().ok_or_else(|| gst_error("Invalid video path"))?;

        // Detect hardware acceleration capabilities for diagnostics only.
        //
        // Modern GStreamer exposes VA-API decoders as codec-specific elements
        // such as `vah264dec`/`vah265dec` instead of the older
        // `vaapidecodebin`. Plain `decodebin` can autoplug those hardware
        // decoders based on rank, so forcing a specific decoder bin here is
        // less portable across distributions.
        let hw_decode = self.config.hw_accel.then(Self::detect_hw_decoder).flatten();
        let decode_element = "decodebin";

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
        let videorate = create_element("videorate")?;
        let videoconvert = create_element("videoconvert")?;
        let videoscale = create_element("videoscale")?;

        let appsink = gst_app::AppSink::builder()
            .name("sink")
            .build();

        // Configure appsink caps for BGRx format.
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRx")
            .field("width", width as i32)
            .field("height", height as i32)
            .field("framerate", gst::Fraction::new(VIDEO_TARGET_FPS as i32, 1))
            .build();

        appsink.set_caps(Some(&caps));
        // Keep only the latest frame. Video wallpapers redraw from the most
        // recent frame, so queueing old frames just adds latency and memory
        // pressure when the compositor cannot consume at the source framerate.
        appsink.set_property("max-buffers", 1u32);
        appsink.set_property("drop", true);
        // We pull frames on demand from next_frame(), so keep appsink unsynced
        // to avoid introducing a second pacing clock besides compositor frames.
        appsink.set_property("sync", false);

        // Add elements to pipeline
        pipeline
            .add_many([
                &filesrc,
                &decodebin,
                &videorate,
                &videoconvert,
                &videoscale,
                appsink.upcast_ref(),
            ])
            .map_err(|e| gst_error(format!("Failed to add elements to pipeline: {}", e)))?;

        // Link static elements
        link_elements(&[&filesrc, &decodebin])?;
        link_elements(&[&videorate, &videoconvert, &videoscale, appsink.upcast_ref()])?;

        // Handle dynamic pad linking from decodebin
        let videorate_weak = videorate.downgrade();
        decodebin.connect_pad_added(move |_src, src_pad| {
            let Some(videorate) = videorate_weak.upgrade() else {
                return;
            };

            let sink_pad = videorate
                .static_pad("sink")
                .expect("videorate has no sink pad");

            if sink_pad.is_linked() {
                return;
            }

            let Some(caps) = src_pad.current_caps() else {
                return;
            };
            let Some(structure) = caps.structure(0) else {
                return;
            };
            let name = structure.name();

            if name.starts_with("video/") {
                if let Err(e) = src_pad.link(&sink_pad) {
                    tracing::error!("Failed to link decodebin pad: {}", e);
                }
            }
        });

        // Store pipeline and appsink
        self.pipeline = Some(pipeline.clone());
        self.appsink = Some(appsink);

        // Set playback speed (clamped to 0.1..=10.0)
        let speed = self.config.clamped_speed();
        if (speed - 1.0).abs() > f64::EPSILON {
            pipeline
                .seek(
                    speed,
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

    /// Detect available hardware decoder families.
    fn detect_hw_decoder() -> Option<HwDecoder> {
        // Check modern VA-API support (Intel, AMD). Fedora and other current
        // distributions expose hardware decoders through the `va` plugin as
        // codec-specific elements instead of the legacy `vaapi` plugin.
        if ["vah264dec", "vah265dec", "vaav1dec", "vavp8dec", "vavp9dec"]
            .into_iter()
            .any(|element| gst::ElementFactory::find(element).is_some())
            || gst::ElementFactory::find("vaapidecodebin").is_some()
        {
            tracing::info!("VA-API hardware decoders available through GStreamer");
            return Some(HwDecoder::VaApi);
        }

        // Check for NVDEC support (NVIDIA)
        if ["nvh264dec", "nvh265dec", "nvav1dec", "nvvp8dec", "nvvp9dec"]
            .into_iter()
            .any(|element| gst::ElementFactory::find(element).is_some())
            || gst::ElementFactory::find("nvdec").is_some()
        {
            tracing::info!("NVDEC hardware decoders available through GStreamer");
            return Some(HwDecoder::Nvdec);
        }

        tracing::info!("No known hardware decoder factories found; decodebin may use software decode");
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
    ///
    /// Part of the video control API. Not yet exposed to config but available
    /// for future interactive controls or power management integration.
    #[allow(dead_code)]
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

        if let Some(appsink) = self.appsink.as_ref() {
            if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::ZERO) {
                if let Some(frame) = sample_to_frame(&sample, &mut self.last_frame) {
                    self.last_frame = Some(frame);
                }
            }
        }

        // Clone only the Arc, not the full image. At ultrawide resolutions a
        // single full-frame clone costs tens of megabytes, so shared ownership
        // matters for 60 FPS video.
        if let Some(payload) = self.last_frame.as_ref() {
            Ok(Frame {
                payload: payload.clone(),
                timestamp: Instant::now(),
            })
        } else {
            // No frame yet, return black frame
            let (width, height) = self.target_size.unwrap_or(crate::source::FALLBACK_RESOLUTION);
            let black = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 255]));
            Ok(Frame {
                payload: FramePayload::Image(Arc::new(DynamicImage::ImageRgba8(black))),
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
        self.last_frame = None;
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

fn sample_to_frame(
    sample: &gst::Sample,
    reusable_frame: &mut Option<FramePayload>,
) -> Option<FramePayload> {
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let caps = sample.caps()?;
    let structure = caps.structure(0)?;

    let width = structure.get::<i32>("width").ok()? as u32;
    let height = structure.get::<i32>("height").ok()? as u32;
    let stride = structure
        .get::<i32>("stride")
        .ok()
        .map(|s| s as usize)
        .unwrap_or(width as usize * 4);
    let frame_size = stride.checked_mul(height as usize)?;

    if map.as_slice().len() < frame_size {
        return None;
    }

    let source = &map.as_slice()[..frame_size];

    let raw = match reusable_frame.take() {
        Some(FramePayload::Bgrx { mut data, .. }) if data.len() == frame_size => {
            if let Some(bytes) = Arc::get_mut(&mut data) {
                bytes.copy_from_slice(source);
                data
            } else {
                Arc::<[u8]>::from(source.to_vec())
            }
        }
        frame => {
            *reusable_frame = frame;
            Arc::<[u8]>::from(source.to_vec())
        }
    };

    Some(FramePayload::Bgrx {
        data: raw,
        width,
        height,
        stride,
    })
}

#[allow(dead_code)]
fn sample_to_image(sample: &gst::Sample) -> Option<Arc<DynamicImage>> {
    let payload = sample_to_frame(sample, &mut None)?;
    let FramePayload::Bgrx {
        data,
        width,
        height,
        stride,
    } = payload
    else {
        return None;
    };

    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for row in 0..height as usize {
        let src_start = row * stride;
        let src_end = src_start + width as usize * 4;
        let dst_start = row * width as usize * 4;
        let src_row = &data[src_start..src_end];
        let dst_row = &mut rgba[dst_start..dst_start + width as usize * 4];

        for (src, dst) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
            let [b, g, r, _x] = src else { unreachable!() };
            dst.copy_from_slice(&[*r, *g, *b, 255]);
        }
    }

    let img_buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba)?;
    Some(Arc::new(DynamicImage::ImageRgba8(img_buffer)))
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
    use std::path::PathBuf;

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
