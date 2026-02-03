# Video Wallpaper Support

This document describes the video wallpaper feature implementation in cosmic-bg-ng.

## Feature Overview

cosmic-bg-ng supports animated video wallpapers through GStreamer-based video playback. Videos are decoded, scaled, and rendered as desktop backgrounds with hardware acceleration support when available. The implementation provides:

- **Continuous playback**: Videos loop seamlessly on the desktop
- **Hardware acceleration**: Automatic detection and use of VA-API (Intel/AMD) or NVDEC (NVIDIA)
- **Frame extraction**: Videos decoded to RGBA format for compositor integration
- **Playback control**: Configurable looping, playback speed, and hardware acceleration settings
- **Resource efficiency**: Pipeline cleanup and proper memory management

**Note**: Audio output is not supported. Video wallpapers are rendered as silent visual backgrounds only.

## Supported Formats

All formats supported by your installed GStreamer plugins are supported, including:

- **MP4** (H.264, H.265/HEVC)
- **WebM** (VP8, VP9, AV1)
- **MKV** (Matroska containers)
- **AVI**
- **MOV** (QuickTime)

Format support depends on installed GStreamer plugin packages (base, good, bad, ugly, libav). See [System Requirements](#system-requirements) below.

## Hardware Acceleration

Video decoding can leverage hardware acceleration through two backends:

### VA-API (Video Acceleration API)
- **Supported GPUs**: Intel (HD Graphics, Iris, Arc), AMD (Radeon with AMDGPU driver)
- **Detection**: Checks for `vaapidecodebin` GStreamer element
- **Codecs**: H.264, H.265/HEVC, VP8, VP9, AV1 (GPU-dependent)

### NVDEC (NVIDIA Video Decoder)
- **Supported GPUs**: NVIDIA GPUs with NVDEC support (GTX 900 series and newer)
- **Detection**: Checks for `nvdec` GStreamer element
- **Codecs**: H.264, H.265/HEVC, VP8, VP9, AV1 (GPU-dependent)

### Software Fallback
If no hardware decoder is detected or `hw_accel: false`, GStreamer uses software decoding via `decodebin` with CPU-based codecs.

The implementation automatically detects available hardware at pipeline build time. Detection results are logged when running with `RUST_LOG=cosmic_bg=info`.

## VideoSource API

### Struct Definition

```rust
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
```

### VideoConfig Options

```rust
pub struct VideoConfig {
    /// Path to the video file
    pub path: PathBuf,

    /// Whether to loop playback (default: true)
    pub loop_playback: bool,

    /// Playback speed multiplier (default: 1.0)
    /// Values < 1.0 = slow motion, > 1.0 = fast forward
    pub playback_speed: f64,

    /// Whether to use hardware acceleration (default: true)
    pub hw_accel: bool,
}
```

**Defaults**:
- `loop_playback: true` - Videos restart from beginning when they reach the end
- `playback_speed: 1.0` - Normal playback speed
- `hw_accel: true` - Hardware acceleration enabled if available

### WallpaperSource Implementation

`VideoSource` implements the `WallpaperSource` trait from `src/source.rs`:

```rust
pub trait WallpaperSource: Send + Sync {
    /// Get the next frame to render
    fn next_frame(&mut self) -> Result<Frame, SourceError>;

    /// Duration until next frame should be rendered
    fn frame_duration(&self) -> Duration;

    /// Whether this source requires continuous rendering
    fn is_animated(&self) -> bool;  // Returns true for VideoSource

    /// Prepare source for rendering at given dimensions
    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError>;

    /// Release resources when no longer needed
    fn release(&mut self);

    /// Get a description of this source for debugging
    fn description(&self) -> String;
}
```

**Key methods**:

- **`prepare(width, height)`**: Builds GStreamer pipeline configured for target output dimensions. Must be called before `next_frame()`.
- **`next_frame()`**: Returns the current video frame as a `DynamicImage` wrapped in a `Frame` struct. Automatically starts playback on first call. Returns black frame if no frame is available yet.
- **`frame_duration()`**: Returns ~33ms (approximately 30fps) for frame timing hints.
- **`is_animated()`**: Returns `true` to indicate continuous rendering is required.
- **`release()`**: Stops pipeline, sets state to NULL, and cleans up resources. Called automatically on `Drop`.

## GStreamer Pipeline Architecture

The video pipeline consists of five linked elements:

```
filesrc → decodebin → videoconvert → videoscale → appsink
          (hw accel)    (to RGBA)     (resize)    (extract)
```

### Pipeline Elements

1. **`filesrc`**: Reads video file from disk
   - Configured with `location` property pointing to video path

2. **`decodebin`** (or `vaapidecodebin`/`nvdec`): Decodes video stream
   - Automatically detects codec and creates appropriate decoder
   - Uses hardware decoder (`vaapidecodebin` or `nvdec`) if available and enabled
   - Dynamically pads are connected via `pad-added` signal callback

3. **`videoconvert`**: Converts decoded frames to RGBA format
   - Target format: `video/x-raw,format=RGBA`
   - Ensures consistent pixel format for compositor

4. **`videoscale`**: Scales video to target output dimensions
   - Dimensions set from `prepare(width, height)` call
   - Scaling algorithm handled by GStreamer

5. **`appsink`**: Extracts frames into application memory
   - Configured caps: `video/x-raw,format=RGBA,width={w},height={h}`
   - `sync: false` - No clock synchronization (async wallpaper rendering)
   - `emit-signals: true` - Enables callback-based frame extraction

### Dynamic Pad Linking

`decodebin` creates output pads dynamically once the codec is detected. The implementation connects the `pad-added` signal to link video streams to `videoconvert`:

```rust
decodebin.connect_pad_added(move |_src, src_pad| {
    // Check if pad is video stream
    if pad_name.starts_with("video/") {
        // Link to videoconvert sink pad
        src_pad.link(&sink_pad)
    }
});
```

This handles various container formats that may have multiple streams.

## Frame Extraction Process

Frames are extracted via `AppSinkCallbacks` registered on the `appsink` element:

1. **Sample arrival**: `new_sample` callback invoked when frame is ready
2. **Buffer mapping**: Frame buffer mapped to readable memory (`buffer.map_readable()`)
3. **Metadata extraction**: Width and height extracted from sample caps
4. **Image creation**: Raw RGBA bytes wrapped in `ImageBuffer<Rgba<u8>>`
5. **Frame storage**: Image stored in `Arc<Mutex<Option<DynamicImage>>>` for thread-safe access
6. **Frame retrieval**: `next_frame()` clones current frame for rendering

### Thread Safety

- GStreamer pipeline runs in separate threads managed by GStreamer runtime
- `Arc<Mutex<>>` wrapper provides thread-safe access to current frame
- Frame buffer is cloned on `next_frame()` to avoid blocking GStreamer callbacks

### Memory Management

- Only the most recent frame is kept in memory
- Old frames are dropped when new frames arrive
- Pipeline resources are cleaned up in `Drop` implementation
- No frame buffering beyond GStreamer's internal queues (typically 2-5 frames)

## Integration Status

### Current State

The video wallpaper implementation in `src/video.rs` is **functionally complete** but **not yet integrated** into the configuration system.

**What exists**:
- ✅ `VideoSource` struct with full `WallpaperSource` trait implementation
- ✅ `VideoConfig` configuration struct
- ✅ GStreamer pipeline with hardware acceleration
- ✅ Frame extraction and format conversion
- ✅ Playback control (play, pause, loop, speed)
- ✅ Resource cleanup and error handling
- ✅ Unit tests

**What's missing**:
- ❌ `Source::Video` variant in `config/src/lib.rs` (line 159)
- ❌ Serialization/deserialization derives on `VideoConfig`
- ❌ Integration with `src/wallpaper.rs` wallpaper loading logic
- ❌ UI support in COSMIC settings/background configuration

### Why It's Not Enabled

The `Source` enum in `config/src/lib.rs` currently supports:

```rust
pub enum Source {
    Path(PathBuf),     // Static images and slideshows
    Color(Color),      // Solid colors and gradients
    Shader(ShaderConfig), // GPU shader wallpapers
    // Source::Video(VideoConfig) not yet added
}
```

Adding `Source::Video(VideoConfig)` requires:

1. **Config changes**: Add variant to `Source` enum with serialization support
2. **Wallpaper logic**: Update `src/wallpaper.rs` to instantiate `VideoSource` when config has `Source::Video`
3. **Settings UI**: Add video file picker and video-specific options to COSMIC settings
4. **State persistence**: Ensure video playback state (current position) is handled correctly across restarts

## Future Work

### Planned Enhancements

- [ ] **Configuration integration**: Add `Source::Video` variant and wire up wallpaper loading
- [ ] **Settings UI**: Video file browser and configuration options
- [ ] **Power management**: Pause video playback when system is idle or screen locked
- [ ] **Frame rate control**: Configurable target framerate (currently ~30fps)
- [ ] **Codec preferences**: Allow forcing specific decoders for troubleshooting
- [ ] **Multiple videos**: Playlist support with cross-fade transitions

### Possible Future Features

- [ ] **Audio support**: Optional audio output for video wallpapers (with mute control)
- [ ] **Video effects**: Filters, color grading, or visual effects on playback
- [ ] **Per-output videos**: Different videos on different monitors
- [ ] **Subtitle rendering**: Display subtitles embedded in video files
- [ ] **Network streams**: Support for RTSP/HTTP video streams
- [ ] **Video layers**: Compositing multiple video sources
- [ ] **Timestamp control**: Start at specific position, chapter markers

## System Requirements

### Build Dependencies

```toml
[dependencies]
gstreamer = "0.23"
gstreamer-app = "0.23"
gstreamer-video = "0.23"
```

**System packages** (build-time):
- GStreamer 1.20+ development headers
- gstreamer-app development headers
- gstreamer-video development headers

### Runtime Dependencies

**Core**:
- GStreamer 1.20 or newer
- GStreamer plugins: base, good, bad, ugly, libav

**Hardware acceleration** (optional but recommended):
- **VA-API**: `libva` + GPU drivers with VA-API support
- **NVDEC**: NVIDIA proprietary drivers (version 450+)

### Installation by Distribution

#### NixOS

The `flake.nix` includes all necessary dependencies:

```nix
buildInputs = with pkgs; [
  gstreamer
  gst-plugins-base
  gst-plugins-good
  gst-plugins-bad
  gst-plugins-ugly
  gst-libav
  libva  # VA-API support
];
```

Hardware acceleration drivers:
```nix
# Intel/AMD VA-API
hardware.opengl.extraPackages = [ pkgs.intel-media-driver pkgs.vaapiIntel ];

# NVIDIA (proprietary)
services.xserver.videoDrivers = [ "nvidia" ];
```

#### Arch Linux

```bash
# Core packages
sudo pacman -S gstreamer gst-plugins-base gst-plugins-good \
               gst-plugins-bad gst-plugins-ugly gst-libav

# Hardware acceleration
sudo pacman -S libva intel-media-driver  # Intel
sudo pacman -S libva mesa-vdpau          # AMD
sudo pacman -S nvidia nvidia-utils       # NVIDIA
```

#### Ubuntu/Debian

```bash
# Core packages
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
                 gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
                 gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
                 gstreamer1.0-libav

# Hardware acceleration
sudo apt install libva-dev va-driver-all  # VA-API (Intel/AMD)
sudo apt install nvidia-driver            # NVIDIA
```

#### Fedora

```bash
# Core packages
sudo dnf install gstreamer1-devel gstreamer1-plugins-base-devel \
                 gstreamer1-plugins-base gstreamer1-plugins-good \
                 gstreamer1-plugins-bad-free gstreamer1-plugins-ugly \
                 gstreamer1-libav

# Hardware acceleration
sudo dnf install libva-intel-driver mesa-va-drivers  # Intel/AMD
sudo dnf install nvidia-driver                       # NVIDIA (from RPM Fusion)
```

## Troubleshooting

### Check Hardware Acceleration

Verify VA-API or NVDEC elements are available:

```bash
# VA-API
gst-inspect-1.0 vaapidecodebin

# NVDEC
gst-inspect-1.0 nvdec
```

If elements are missing, install hardware acceleration drivers.

### Debug Logging

Enable cosmic-bg debug logging to see hardware acceleration detection and pipeline events:

```bash
RUST_LOG=cosmic_bg=debug cosmic-bg
```

Look for lines like:
```
INFO cosmic_bg::video: VA-API hardware acceleration available
INFO cosmic_bg::video: Building video pipeline hw_accel=Some(VaApi) decoder="vaapidecodebin"
DEBUG cosmic_bg::video: Video playback started
```

### Video Not Playing

1. **Test video file compatibility**:
   ```bash
   gst-discoverer-1.0 /path/to/video.mp4
   ```

2. **Test with simple pipeline**:
   ```bash
   gst-play-1.0 /path/to/video.mp4
   ```

3. **Check cosmic-bg logs**:
   ```bash
   journalctl --user -u cosmic-bg -f
   ```

4. **Verify GStreamer plugins**:
   ```bash
   gst-inspect-1.0 decodebin
   ```

### High CPU Usage

1. **Verify hardware acceleration is enabled**: Check logs for "software decode" fallback
2. **Install hardware decoder drivers**: See [System Requirements](#system-requirements)
3. **Test hardware decoding**:
   ```bash
   # VA-API test
   gst-launch-1.0 filesrc location=video.mp4 ! vaapidecodebin ! fakesink

   # NVDEC test
   gst-launch-1.0 filesrc location=video.mp4 ! nvdec ! fakesink
   ```
4. **Reduce video resolution**: Use lower resolution videos (1080p instead of 4K)
5. **Lower playback speed**: Try `playback_speed: 0.5` to reduce decode load

### Pipeline Errors

Common GStreamer error messages:

- **"No URI handler implemented"**: Video path is invalid
- **"Could not link decodebin"**: Codec not supported (install more plugins)
- **"Internal data stream error"**: Corrupted video file
- **"Resource not found"**: File doesn't exist or permission denied

Check GStreamer element availability:
```bash
gst-inspect-1.0 | grep -i decode
```

## References

- [GStreamer Documentation](https://gstreamer.freedesktop.org/documentation/)
- [gstreamer-rs Rust Bindings](https://gitlab.freedesktop.org/gstreamer/gstreamer-rs)
- [GStreamer Plugin Reference](https://gstreamer.freedesktop.org/documentation/plugins_doc.html)
- [VA-API (Video Acceleration API)](https://github.com/intel/libva)
- [NVDEC (NVIDIA Video Decoder)](https://developer.nvidia.com/nvidia-video-codec-sdk)
- [cosmic-bg-ng Repository](https://github.com/cosmic-utils/cosmic-bg)
