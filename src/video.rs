// SPDX-License-Identifier: MPL-2.0

use crate::source::{Frame, FramePayload, SourceError, WallpaperSource};
use cosmic_ext_bg_config::VideoConfig;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use image::{DynamicImage, ImageBuffer, Rgba};
use std::{
    ffi::OsString,
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

fn frame_duration_from_fps(fps: u32) -> Duration {
    Duration::from_micros(1_000_000 / u64::from(fps))
}

fn frame_duration_from_caps(caps: &gst::CapsRef) -> Option<Duration> {
    let structure = caps.structure(0)?;
    let framerate = structure.get::<gst::Fraction>("framerate").ok()?;
    let numer = framerate.numer();
    let denom = framerate.denom();

    if numer <= 0 || denom <= 0 {
        return None;
    }

    let micros = 1_000_000_u64
        .checked_mul(denom as u64)?
        .checked_div(numer as u64)?;

    Some(Duration::from_micros(micros.max(1)))
}

fn clamp_fps(fps: u32) -> u32 {
    fps.clamp(1, 240)
}

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

fn video_caps(target_size: Option<(u32, u32)>, fps_limit: Option<u32>) -> gst::Caps {
    let mut caps = gst::Caps::builder("video/x-raw").field("format", "BGRx");

    if let Some((width, height)) = target_size {
        caps = caps
            .field("width", width as i32)
            .field("height", height as i32);
    }

    if let Some(fps) = fps_limit {
        caps = caps.field("framerate", gst::Fraction::new(clamp_fps(fps) as i32, 1));
    }

    caps.build()
}

/// Video wallpaper source with GStreamer backend
pub struct VideoSource {
    config: VideoConfig,
    backend: VideoBackend,
    pipeline: Option<gst::Pipeline>,
    appsink: Option<gst_app::AppSink>,
    ffmpeg: Option<FfmpegProcess>,
    ffmpeg_ended: bool,
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

        let frame_duration = frame_duration_from_fps(config.target_fps());

        Ok(Self {
            config,
            backend: VideoBackend::GStreamer,
            pipeline: None,
            appsink: None,
            ffmpeg: None,
            ffmpeg_ended: false,
            last_frame: None,
            target_size: None,
            frame_duration,
            is_playing: false,
            is_prepared: false,
        })
    }

    fn build_backend(&mut self, target_size: Option<(u32, u32)>) -> Result<(), SourceError> {
        if should_try_ffmpeg(&self.config.path) {
            match FfmpegProcess::spawn(&self.config, target_size) {
                Ok(process) => {
                    self.frame_duration = process.frame_duration;
                    self.ffmpeg = Some(process);
                    self.ffmpeg_ended = false;
                    self.backend = VideoBackend::Ffmpeg;
                    tracing::info!(path = %self.config.path.display(), "Using ffmpeg video backend");
                    return Ok(());
                }
                Err(error) => {
                    tracing::warn!(?error, path = %self.config.path.display(), "Falling back to GStreamer video backend");
                }
            }
        }

        self.backend = VideoBackend::GStreamer;
        self.build_pipeline(target_size)
    }

    /// Build the GStreamer pipeline for video playback
    fn build_pipeline(&mut self, target_size: Option<(u32, u32)>) -> Result<(), SourceError> {
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

        let fps_limit = self.config.fps_limit.map(clamp_fps);
        tracing::info!(
            hw_accel = ?hw_decode,
            decoder = decode_element,
            playback_rate = if fps_limit.is_some() { "fps_limit" } else { "source_rate" },
            fps_limit,
            "Building video pipeline"
        );

        // Create pipeline elements
        let pipeline = gst::Pipeline::new();

        let filesrc = gst::ElementFactory::make("filesrc")
            .property("location", path)
            .build()
            .map_err(|e| gst_error(format!("Failed to create filesrc: {}", e)))?;

        let decodebin = create_element(decode_element)?;
        let decode_queue = create_element("queue")?;
        decode_queue.set_property("max-size-buffers", 4u32);
        decode_queue.set_property("max-size-bytes", 0u32);
        decode_queue.set_property("max-size-time", 0u64);
        let videorate = if fps_limit.is_some() {
            Some(create_element("videorate")?)
        } else {
            None
        };
        let videoconvert = create_element("videoconvert")?;
        let videoscale = create_element("videoscale")?;

        let appsink = gst_app::AppSink::builder()
            .name("sink")
            .build();

        // Configure appsink caps for BGRx format.
        let caps = video_caps(target_size, self.config.fps_limit);

        appsink.set_caps(Some(&caps));
        // Keep a tiny sink queue so near-ready frames are not discarded before
        // compositor-driven pulls, without accumulating meaningful latency.
        appsink.set_property("max-buffers", 2u32);
        appsink.set_property("drop", true);
        // We pull frames on demand from next_frame(), so keep appsink unsynced
        // to avoid introducing a second pacing clock besides compositor frames.
        appsink.set_property("sync", false);

        // Add elements to pipeline
        pipeline
            .add_many([
                &filesrc,
                &decodebin,
                &decode_queue,
                &videoconvert,
                &videoscale,
                appsink.upcast_ref(),
            ])
            .map_err(|e| gst_error(format!("Failed to add elements to pipeline: {}", e)))?;
        if let Some(videorate) = videorate.as_ref() {
            pipeline
                .add(videorate)
                .map_err(|e| gst_error(format!("Failed to add videorate to pipeline: {}", e)))?;
        }

        // Link static elements
        link_elements(&[&filesrc, &decodebin])?;
        if let Some(videorate) = videorate.as_ref() {
            link_elements(&[
                &decode_queue,
                videorate,
                &videoconvert,
                &videoscale,
                appsink.upcast_ref(),
            ])?;
        } else {
            link_elements(&[
                &decode_queue,
                &videoconvert,
                &videoscale,
                appsink.upcast_ref(),
            ])?;
        }

        // Handle dynamic pad linking from decodebin
        let video_pad_target = decode_queue.downgrade();
        decodebin.connect_pad_added(move |_src, src_pad| {
            let Some(video_pad_target) = video_pad_target.upgrade() else {
                return;
            };

            let sink_pad = video_pad_target
                .static_pad("sink")
                .expect("video pad target has no sink pad");

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
        if matches!(self.backend, VideoBackend::Ffmpeg) {
            self.is_playing = true;
            return Ok(());
        }

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

    /// Check non-blocking GStreamer bus messages for diagnostics and EOS handling.
    fn handle_bus_messages(&mut self) -> Result<(), SourceError> {
        let Some(ref pipeline) = self.pipeline else {
            return Ok(());
        };

        let Some(bus) = pipeline.bus() else {
            return Err(gst_error("No bus available"));
        };

        while let Some(msg) = bus.pop_filtered(&[
            gst::MessageType::Error,
            gst::MessageType::Warning,
            gst::MessageType::Eos,
        ]) {
            match msg.view() {
                gst::MessageView::Error(error) => {
                    tracing::error!(
                        error = %error.error(),
                        debug = ?error.debug(),
                        "GStreamer video pipeline error"
                    );
                }
                gst::MessageView::Warning(warning) => {
                    tracing::warn!(
                        warning = %warning.error(),
                        debug = ?warning.debug(),
                        "GStreamer video pipeline warning"
                    );
                }
                gst::MessageView::Eos(_) => {
                    if self.config.loop_playback {
                        tracing::debug!("Video reached end, looping");
                        pipeline
                            .seek_simple(
                                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                                gst::ClockTime::from_seconds(0),
                            )
                            .ok();
                    } else {
                        tracing::debug!("Video reached end");
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn next_ffmpeg_frame(&mut self) -> Option<FramePayload> {
        let Some(process) = self.ffmpeg.as_mut() else {
            return None;
        };

        let mut respawn_target = None;
        let latest = process.latest_frame.lock().ok().and_then(|mut frame| {
            frame.take().map(|data| FramePayload::Bgrx {
                data,
                width: process.width,
                height: process.height,
                stride: process.stride,
            })
        });

        if process.ended.load(Ordering::Acquire) {
            if self.config.loop_playback {
                tracing::warn!(path = %self.config.path.display(), "ffmpeg video reader ended unexpectedly; respawning");
                respawn_target = Some(process.target_size);
            } else {
                tracing::debug!(path = %self.config.path.display(), "ffmpeg video reached end");
                self.ffmpeg_ended = true;
                self.last_frame = None;
                self.ffmpeg = None;
                return None;
            }
        }

        if let Some(target_size) = respawn_target {
            match FfmpegProcess::spawn(&self.config, target_size) {
                Ok(restarted) => {
                    self.frame_duration = restarted.frame_duration;
                    self.ffmpeg = Some(restarted);
                    self.ffmpeg_ended = false;
                }
                Err(error) => {
                    tracing::error!(?error, path = %self.config.path.display(), "Failed to respawn ffmpeg video backend");
                    self.ffmpeg_ended = true;
                    self.last_frame = None;
                    self.ffmpeg = None;
                    return None;
                }
            }
        }

        latest
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

        if self.ffmpeg_ended {
            self.last_frame = None;
        } else if matches!(self.backend, VideoBackend::Ffmpeg) {
            if let Some(frame) = self.next_ffmpeg_frame() {
                self.last_frame = Some(frame);
            }
        } else {
            // Check for pipeline diagnostics and end-of-stream handling.
            self.handle_bus_messages()?;

            if let Some(appsink) = self.appsink.as_ref() {
                let timeout = if self.last_frame.is_some() {
                    gst::ClockTime::from_mseconds(3)
                } else {
                    gst::ClockTime::from_mseconds(
                        self.frame_duration.min(Duration::from_millis(50)).as_millis() as u64,
                    )
                };

                if let Some(sample) = appsink.try_pull_sample(timeout) {
                    if self.config.fps_limit.is_none() {
                        if let Some(caps) = sample.caps() {
                            if let Some(frame_duration) = frame_duration_from_caps(caps) {
                                self.frame_duration = frame_duration;
                            }
                        }
                    }

                    if let Some(frame) = sample_to_frame(&sample, &mut self.last_frame) {
                        self.last_frame = Some(frame);
                    }
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
                is_placeholder: false,
            })
        } else {
            // No frame yet, return black frame
            let (width, height) = self.target_size.unwrap_or(crate::source::FALLBACK_RESOLUTION);
            let black = ImageBuffer::from_pixel(width, height, Rgba([0, 0, 0, 255]));
            Ok(Frame::placeholder(FramePayload::Image(Arc::new(
                DynamicImage::ImageRgba8(black),
            ))))
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

        // Build backend if not already built
        if self.pipeline.is_none() && self.ffmpeg.is_none() {
            self.build_backend(Some((width, height)))?;
        }

        self.is_prepared = true;
        Ok(())
    }

    fn prepare_unscaled(&mut self) -> Result<(), SourceError> {
        self.target_size = Some(crate::source::FALLBACK_RESOLUTION);

        if self.pipeline.is_none() && self.ffmpeg.is_none() {
            self.build_backend(None)?;
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
        self.ffmpeg = None;
        self.ffmpeg_ended = false;
        self.last_frame = None;
        self.backend = VideoBackend::GStreamer;
        self.is_playing = false;
        self.is_prepared = false;

        tracing::debug!("Video source released");
    }

    fn description(&self) -> String {
        format!(
            "Video: {} (loop: {}, hw_accel: {}, fps: {})",
            self.config.path.display(),
            self.config.loop_playback,
            self.config.hw_accel,
            self.config.target_fps()
        )
    }
}

impl std::fmt::Debug for VideoSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoSource")
            .field("config", &self.config)
            .field("backend", &self.backend)
            .field("ffmpeg_ended", &self.ffmpeg_ended)
            .field("target_size", &self.target_size)
            .field("frame_duration", &self.frame_duration)
            .field("is_playing", &self.is_playing)
            .field("is_prepared", &self.is_prepared)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoBackend {
    GStreamer,
    Ffmpeg,
}

struct FfmpegProcess {
    width: u32,
    height: u32,
    stride: usize,
    target_size: Option<(u32, u32)>,
    frame_duration: Duration,
    latest_frame: Arc<Mutex<Option<Arc<[u8]>>>>,
    ended: Arc<AtomicBool>,
    child: Child,
    reader: Option<JoinHandle<()>>,
}

impl FfmpegProcess {
    fn spawn(config: &VideoConfig, target_size: Option<(u32, u32)>) -> Result<Self, SourceError> {
        let metadata = probe_video(&config.path)?;
        let (width, height) = target_size.unwrap_or((metadata.width, metadata.height));
        let stride = width as usize * 4;
        let frame_size = stride
            .checked_mul(height as usize)
            .ok_or_else(|| gst_error("ffmpeg frame dimensions overflow"))?;
        let args = ffmpeg_args(config, target_size);

        let mut child = Command::new("ffmpeg")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| gst_error(format!("Failed to spawn ffmpeg: {}", e)))?;

        let Some(stdout) = child.stdout.take() else {
            kill_and_wait(&mut child);
            return Err(gst_error("ffmpeg stdout was not piped"));
        };
        let latest_frame = Arc::new(Mutex::new(None));
        let ended = Arc::new(AtomicBool::new(false));
        let reader_latest_frame = Arc::clone(&latest_frame);
        let reader_ended = Arc::clone(&ended);
        let reader = match thread::Builder::new()
            .name("cosmic-ext-bg-ffmpeg-reader".to_string())
            .spawn(move || read_ffmpeg_frames(stdout, frame_size, reader_latest_frame, reader_ended))
        {
            Ok(reader) => reader,
            Err(error) => {
                kill_and_wait(&mut child);
                return Err(gst_error(format!(
                    "Failed to start ffmpeg reader thread: {}",
                    error
                )));
            }
        };

        Ok(Self {
            width,
            height,
            stride,
            target_size,
            frame_duration: adjusted_frame_duration(config, metadata.frame_duration),
            latest_frame,
            ended,
            child,
            reader: Some(reader),
        })
    }
}

fn kill_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for FfmpegProcess {
    fn drop(&mut self) {
        kill_and_wait(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoMetadata {
    width: u32,
    height: u32,
    frame_duration: Duration,
}

fn should_try_ffmpeg(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

fn ffmpeg_args(config: &VideoConfig, target_size: Option<(u32, u32)>) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-nostdin"),
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("error"),
    ];

    // Note: ffmpeg's native software H.264/AV1 decoders are used deliberately.
    // Benchmarking 4K H.264 showed `-hwaccel auto` was slower end-to-end than
    // software decode here because the GPU->system-memory frame download costs
    // more than it saves for the rawvideo pipe. Software decode already sustains
    // well above real-time, so the bottleneck is not decode.

    if config.loop_playback {
        args.extend([OsString::from("-stream_loop"), OsString::from("-1")]);
    }

    args.extend([OsString::from("-re"), OsString::from("-i")]);
    args.push(config.path.as_os_str().to_os_string());

    if config.fps_limit.is_some() || target_size.is_some() {
        args.extend([
            OsString::from("-vf"),
            OsString::from(ffmpeg_filter(config.fps_limit, target_size)),
        ]);
    }

    args.extend([
        OsString::from("-an"),
        OsString::from("-f"),
        OsString::from("rawvideo"),
        OsString::from("-pix_fmt"),
        OsString::from("bgr0"),
        OsString::from("pipe:1"),
    ]);

    args
}

fn ffmpeg_filter(fps_limit: Option<u32>, target_size: Option<(u32, u32)>) -> String {
    let mut filters = Vec::new();
    if let Some(fps) = fps_limit {
        filters.push(format!("fps={}", clamp_fps(fps)));
    }
    if let Some((width, height)) = target_size {
        filters.push(format!("scale={}:{}", width, height));
    }
    filters.join(",")
}

fn probe_video(path: &Path) -> Result<VideoMetadata, SourceError> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| gst_error(format!("Failed to run ffprobe: {}", e)))?;

    if !output.status.success() {
        return Err(gst_error(format!(
            "ffprobe failed for {} with status {}",
            path.display(),
            output.status
        )));
    }

    parse_ffprobe_output(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ffprobe_output(output: &str) -> Result<VideoMetadata, SourceError> {
    let mut width = None;
    let mut height = None;
    let mut frame_duration = None;

    for line in output.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        match key {
            "width" => width = value.parse::<u32>().ok(),
            "height" => height = value.parse::<u32>().ok(),
            "r_frame_rate" => frame_duration = parse_frame_duration_fraction(value),
            _ => {}
        }
    }

    let width = width.ok_or_else(|| gst_error("ffprobe did not report video width"))?;
    let height = height.ok_or_else(|| gst_error("ffprobe did not report video height"))?;
    let frame_duration = frame_duration.unwrap_or_else(|| frame_duration_from_fps(60));

    if width == 0 || height == 0 {
        return Err(gst_error("ffprobe reported invalid video dimensions"));
    }

    Ok(VideoMetadata {
        width,
        height,
        frame_duration,
    })
}

fn parse_frame_duration_fraction(value: &str) -> Option<Duration> {
    let (numer, denom) = value.split_once('/')?;
    let numer = numer.parse::<u64>().ok()?;
    let denom = denom.parse::<u64>().ok()?;
    if numer == 0 || denom == 0 {
        return None;
    }

    let micros = 1_000_000_u64.checked_mul(denom)?.checked_div(numer)?;
    Some(Duration::from_micros(micros.max(1)))
}

fn adjusted_frame_duration(config: &VideoConfig, source_frame_duration: Duration) -> Duration {
    let base = config
        .fps_limit
        .map(clamp_fps)
        .map(frame_duration_from_fps)
        .unwrap_or(source_frame_duration);
    let speed = config.clamped_speed();
    Duration::from_secs_f64((base.as_secs_f64() / speed).max(0.001))
}

fn read_ffmpeg_frames(
    mut stdout: impl Read,
    frame_size: usize,
    latest_frame: Arc<Mutex<Option<Arc<[u8]>>>>,
    ended: Arc<AtomicBool>,
) {
    let mut produced: u64 = 0;
    let mut window_start = Instant::now();
    loop {
        let mut frame = vec![0; frame_size];
        if stdout.read_exact(&mut frame).is_err() {
            break;
        }
        produced += 1;
        // Report the real decode + pipe throughput periodically so we can tell a
        // decode/pipe bottleneck (low produced fps) from a render bottleneck.
        if produced % 48 == 0 {
            let elapsed = window_start.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                tracing::debug!(
                    produced_fps = 48.0 / elapsed,
                    frame_bytes = frame_size,
                    "ffmpeg decode throughput"
                );
            }
            window_start = Instant::now();
        }
        let Ok(mut latest) = latest_frame.lock() else {
            break;
        };
        *latest = Some(Arc::<[u8]>::from(frame));
    }
    ended.store(true, Ordering::Release);
}

#[cfg(test)]
fn read_ffmpeg_frames_for_test(frames: &[&[u8]], frame_size: usize) -> Option<Vec<u8>> {
    let latest_frame = Arc::new(Mutex::new(None));
    let ended = Arc::new(AtomicBool::new(false));
    let mut bytes = Vec::new();
    for frame in frames {
        bytes.extend_from_slice(frame);
    }

    read_ffmpeg_frames(&bytes[..], frame_size, Arc::clone(&latest_frame), ended);

    latest_frame
        .lock()
        .ok()
        .and_then(|frame| frame.as_ref().map(|frame| frame.to_vec()))
}

#[cfg(test)]
fn os_args_contain_pair(args: &[OsString], name: &str, value: &str) -> bool {
    args.windows(2).any(|window| {
        window[0].as_os_str() == name && window[1].as_os_str() == value
    })
}

#[cfg(test)]
fn os_args_contain(args: &[OsString], value: &str) -> bool {
    args.iter().any(|arg| arg.as_os_str() == value)
}

#[cfg(test)]
fn os_args_position(args: &[OsString], value: &str) -> Option<usize> {
    args.iter().position(|arg| arg.as_os_str() == value)
}

#[cfg(test)]
fn os_args_last(args: &[OsString]) -> Option<&str> {
    args.last().and_then(|arg| arg.to_str())
}

#[cfg(test)]
fn take_ffmpeg_frame_for_test(source: &mut VideoSource) -> Option<FramePayload> {
    source.next_ffmpeg_frame()
}

#[cfg(test)]
fn test_ffmpeg_process(
    config: &VideoConfig,
    latest_frame: Arc<Mutex<Option<Arc<[u8]>>>>,
    ended: Arc<AtomicBool>,
) -> FfmpegProcess {
    FfmpegProcess {
        width: 1,
        height: 1,
        stride: 4,
        target_size: Some((1, 1)),
        frame_duration: adjusted_frame_duration(config, frame_duration_from_fps(60)),
        latest_frame,
        ended,
        child: Command::new("true").spawn().unwrap(),
        reader: None,
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
        assert_eq!(config.fps_limit, None);
        assert_eq!(config.target_fps(), 60);
    }

    #[test]
    fn test_video_config_fps_limit() {
        let config = VideoConfig {
            fps_limit: Some(30),
            ..Default::default()
        };

        assert_eq!(config.target_fps(), 30);
    }

    #[test]
    fn test_video_config_fps_limit_clamps_to_safe_range() {
        let too_low = VideoConfig {
            fps_limit: Some(0),
            ..Default::default()
        };
        let too_high = VideoConfig {
            fps_limit: Some(999),
            ..Default::default()
        };

        assert_eq!(too_low.target_fps(), 1);
        assert_eq!(too_high.target_fps(), 240);
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
        assert!(desc.contains("fps: 60"));
    }

    #[test]
    fn test_source_rate_caps_omit_framerate() {
        gst::init().unwrap();

        let caps = video_caps(Some((1920, 1080)), None);
        let structure = caps.structure(0).unwrap();

        assert_eq!(structure.get::<&str>("format").unwrap(), "BGRx");
        assert_eq!(structure.get::<i32>("width").unwrap(), 1920);
        assert_eq!(structure.get::<i32>("height").unwrap(), 1080);
        assert!(structure.get::<gst::Fraction>("framerate").is_err());
    }

    #[test]
    fn test_fps_limited_caps_include_clamped_framerate() {
        gst::init().unwrap();

        let caps = video_caps(None, Some(999));
        let structure = caps.structure(0).unwrap();

        assert_eq!(structure.get::<&str>("format").unwrap(), "BGRx");
        assert!(structure.get::<i32>("width").is_err());
        assert!(structure.get::<i32>("height").is_err());
        assert_eq!(
            structure.get::<gst::Fraction>("framerate").unwrap(),
            gst::Fraction::new(240, 1)
        );
    }

    #[test]
    fn test_frame_duration_from_caps_uses_source_framerate() {
        gst::init().unwrap();

        let caps = gst::Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(24, 1))
            .build();

        assert_eq!(
            frame_duration_from_caps(&caps),
            Some(Duration::from_micros(41_666))
        );
    }

    #[test]
    fn test_frame_duration_from_caps_ignores_missing_framerate() {
        gst::init().unwrap();

        let caps = gst::Caps::builder("video/x-raw").build();

        assert_eq!(frame_duration_from_caps(&caps), None);
    }

    #[test]
    fn test_should_try_ffmpeg_only_for_mp4() {
        assert!(should_try_ffmpeg(Path::new("/tmp/wallpaper.MP4")));
        assert!(!should_try_ffmpeg(Path::new("/tmp/wallpaper.webm")));
        assert!(!should_try_ffmpeg(Path::new("/tmp/wallpaper")));
    }

    #[test]
    fn test_ffmpeg_args_loop_scaled_fps_limited_bgrx_pipe() {
        let config = VideoConfig {
            path: PathBuf::from("/tmp/video.mp4"),
            loop_playback: true,
            fps_limit: Some(999),
            ..Default::default()
        };

        let args = ffmpeg_args(&config, Some((1920, 1080)));

        assert_eq!(args[0].as_os_str(), "-nostdin");
        assert!(os_args_contain_pair(&args, "-stream_loop", "-1"));
        assert!(os_args_contain_pair(&args, "-vf", "fps=240,scale=1920:1080"));
        assert!(os_args_contain_pair(&args, "-pix_fmt", "bgr0"));
        assert_eq!(os_args_last(&args), Some("pipe:1"));
    }

    #[test]
    fn test_ffmpeg_args_realtime_before_input_and_preserve_path_os_string() {
        let path = PathBuf::from("/tmp/video.mp4");
        let config = VideoConfig {
            path: path.clone(),
            loop_playback: false,
            ..Default::default()
        };

        let args = ffmpeg_args(&config, None);
        let re_index = os_args_position(&args, "-re").unwrap();
        let input_index = os_args_position(&args, "-i").unwrap();

        assert!(re_index < input_index);
        assert_eq!(args.get(input_index + 1).map(OsString::as_os_str), Some(path.as_os_str()));
    }

    #[test]
    fn test_ffmpeg_args_omit_loop_and_filter_when_unlimited_unscaled() {
        let config = VideoConfig {
            path: PathBuf::from("/tmp/video.mp4"),
            loop_playback: false,
            fps_limit: None,
            ..Default::default()
        };

        let args = ffmpeg_args(&config, None);

        assert!(!os_args_contain(&args, "-stream_loop"));
        assert!(!os_args_contain(&args, "-vf"));
        assert!(os_args_contain(&args, "-re"));
    }

    #[test]
    fn test_parse_ffprobe_output_uses_keyed_metadata() {
        let metadata = parse_ffprobe_output("width=3840\nheight=2160\nr_frame_rate=24/1\n").unwrap();

        assert_eq!(metadata.width, 3840);
        assert_eq!(metadata.height, 2160);
        assert_eq!(metadata.frame_duration, Duration::from_micros(41_666));
    }

    #[test]
    fn test_adjusted_frame_duration_uses_fps_limit_and_speed() {
        let config = VideoConfig {
            fps_limit: Some(30),
            playback_speed: 2.0,
            ..Default::default()
        };

        assert_eq!(
            adjusted_frame_duration(&config, Duration::from_millis(100)),
            Duration::from_nanos(16_666_500)
        );
    }

    #[test]
    fn test_ffmpeg_reader_keeps_latest_frame() {
        let latest = read_ffmpeg_frames_for_test(&[b"old1", b"new2", b"last"], 4).unwrap();

        assert_eq!(latest, b"last");
    }

    #[test]
    fn test_ffmpeg_non_loop_eof_clears_stale_frame_and_marks_ended() {
        let latest_frame = Arc::new(Mutex::new(Some(Arc::<[u8]>::from(&b"done"[..]))));
        let ended = Arc::new(AtomicBool::new(true));
        let config = VideoConfig {
            path: PathBuf::from("/tmp/video.mp4"),
            loop_playback: false,
            ..Default::default()
        };
        let process = test_ffmpeg_process(&config, latest_frame, ended);
        let mut source = VideoSource::new(config).unwrap();
        source.backend = VideoBackend::Ffmpeg;
        source.ffmpeg = Some(process);
        source.last_frame = Some(FramePayload::Bgrx {
            data: Arc::<[u8]>::from(&b"stale"[..]),
            width: 1,
            height: 1,
            stride: 4,
        });

        assert!(take_ffmpeg_frame_for_test(&mut source).is_none());
        assert!(source.ffmpeg_ended);
        assert!(source.ffmpeg.is_none());
        assert!(source.last_frame.is_none());
    }

    #[test]
    fn test_ffmpeg_loop_respawn_failure_marks_ended() {
        let latest_frame = Arc::new(Mutex::new(Some(Arc::<[u8]>::from(&b"done"[..]))));
        let ended = Arc::new(AtomicBool::new(true));
        let config = VideoConfig {
            path: PathBuf::from("/tmp/cosmic-ext-bg-missing-respawn-test.mp4"),
            loop_playback: true,
            ..Default::default()
        };
        let process = test_ffmpeg_process(&config, latest_frame, ended);
        let mut source = VideoSource::new(config).unwrap();
        source.backend = VideoBackend::Ffmpeg;
        source.ffmpeg = Some(process);
        source.last_frame = Some(FramePayload::Bgrx {
            data: Arc::<[u8]>::from(&b"stale"[..]),
            width: 1,
            height: 1,
            stride: 4,
        });

        assert!(take_ffmpeg_frame_for_test(&mut source).is_none());
        assert!(source.ffmpeg_ended);
        assert!(source.ffmpeg.is_none());
        assert!(source.last_frame.is_none());

        assert!(take_ffmpeg_frame_for_test(&mut source).is_none());
        assert!(source.ffmpeg.is_none());
    }
}
