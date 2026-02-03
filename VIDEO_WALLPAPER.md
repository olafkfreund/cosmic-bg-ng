# Video Wallpaper Support

This document describes the video wallpaper feature implementation in cosmic-bg-ng.

## Overview

cosmic-bg now supports video wallpapers with hardware acceleration support via GStreamer. Supported formats include MP4, WebM, and any other format supported by your GStreamer plugins.

## Architecture

### Components

1. **VideoSource** (`src/video.rs`): Implements the `WallpaperSource` trait for video playback
2. **VideoConfig** (`src/video.rs`): Internal configuration structure for video settings (not yet exposed in cosmic-bg-config)

### Hardware Acceleration

The implementation automatically detects and uses hardware acceleration when available:

- **VA-API**: For Intel and AMD GPUs (Linux)
- **NVDEC**: For NVIDIA GPUs
- **Software decode**: Fallback when hardware acceleration is unavailable

The detection happens at pipeline build time and is logged for debugging purposes.

## Configuration

### Current Status

**Integration with cosmic-bg-config is pending.** The video wallpaper implementation exists in `src/video.rs` but is not yet exposed as a `Source::Video` variant in `config/src/lib.rs`. The configuration shown below represents the planned/internal API structure.

### VideoConfig Structure (Internal API)

The following configuration structure exists in `src/video.rs`:

```rust
pub struct VideoConfig {
    /// Path to the video file
    pub path: PathBuf,

    /// Whether to loop playback (default: true)
    pub loop_playback: bool,

    /// Playback speed multiplier (default: 1.0)
    pub playback_speed: f64,

    /// Whether to use hardware acceleration (default: true)
    pub hw_accel: bool,
}
```

**Note:** Audio is not supported for desktop wallpapers - only video frames are rendered.

### Planned Configuration Example

Once integrated with cosmic-config, setting a video wallpaper will look like:

```ron
Entry(
    output: "all",
    source: Video(VideoConfig(
        path: "/home/user/Videos/wallpaper.mp4",
        loop_playback: true,
        playback_speed: 1.0,
        hw_accel: true,
    )),
    scaling_mode: Zoom,
    rotation_frequency: 0,
)
```

### Integration TODO

To expose video wallpapers to cosmic-config:

1. Add `Source::Video(VideoConfig)` variant to `config/src/lib.rs` enum (line 159)
2. Update `VideoConfig` in `src/video.rs` to derive `Serialize` and `Deserialize`
3. Integrate `VideoSource` into the wallpaper loading logic in `src/wallpaper.rs`
4. Update UI components in cosmic-settings to support video source selection

## Implementation Status

### Current State

The video wallpaper **core implementation is complete** in `src/video.rs` with full GStreamer integration, hardware acceleration support, and frame extraction. However, **cosmic-config integration is pending** - the `Source::Video` variant does not yet exist in `config/src/lib.rs`, so video wallpapers cannot currently be configured through the COSMIC settings UI.

### What Works Now

The `VideoSource` implementation in `src/video.rs` provides:

- GStreamer-based video playback
- Hardware acceleration detection (VA-API, NVDEC)
- Frame extraction to RGBA format
- Automatic looping
- Playback speed control
- Integration with WallpaperSource trait
- Proper resource cleanup on drop

**Note:** Audio is intentionally not supported - video wallpapers are rendered as silent visual backgrounds only.

### Performance

- Videos are decoded to RGBA format at the target display resolution
- Hardware acceleration significantly reduces CPU usage
- Frame timing is handled by GStreamer's pipeline
- No synchronization to system clock (async rendering for wallpapers)

## System Requirements

### Dependencies

**Build-time:**
- GStreamer 1.20+ development libraries
- gstreamer-app
- gstreamer-video

**Runtime:**
- GStreamer 1.20+
- GStreamer plugins (base, good, bad, ugly, libav)
- VA-API drivers (for Intel/AMD hardware acceleration)
- NVIDIA drivers with NVDEC support (for NVIDIA hardware acceleration)

### NixOS

The flake.nix includes all necessary GStreamer dependencies:

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

### Other Distributions

**Arch Linux:**
```bash
sudo pacman -S gstreamer gst-plugins-base gst-plugins-good \
               gst-plugins-bad gst-plugins-ugly gst-libav libva
```

**Ubuntu/Debian:**
```bash
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
                 gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
                 gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
                 gstreamer1.0-libav libva-dev
```

**Fedora:**
```bash
sudo dnf install gstreamer1-devel gstreamer1-plugins-base-devel \
                 gstreamer1-plugins-base gstreamer1-plugins-good \
                 gstreamer1-plugins-bad-free gstreamer1-plugins-ugly \
                 gstreamer1-libav libva-devel
```

## Troubleshooting

### Hardware Acceleration Not Working

Check if the appropriate GStreamer elements are available:

```bash
# For VA-API
gst-inspect-1.0 vaapi

# For NVDEC
gst-inspect-1.0 nvdec
```

Enable debug logging to see hardware acceleration detection:

```bash
RUST_LOG=cosmic_bg=debug cosmic-bg
```

### Video Not Playing

1. Check video file format compatibility:
```bash
gst-discoverer-1.0 /path/to/video.mp4
```

2. Test with a simple GStreamer pipeline:
```bash
gst-play-1.0 /path/to/video.mp4
```

3. Check cosmic-bg logs:
```bash
journalctl --user -u cosmic-bg -f
```

### High CPU Usage

1. Ensure hardware acceleration is enabled (`hw_accel: true`)
2. Verify hardware decoders are available (see above)
3. Check video resolution - consider using lower resolution videos
4. Try reducing `playback_speed` or setting `loop_playback: false`

## Implementation Details

### Frame Extraction

Videos are decoded to RGBA format at the target display resolution using GStreamer's `videoconvert` and `videoscale` elements. Frames are extracted via `AppSink` and converted to `image::DynamicImage` for consistency with the existing wallpaper pipeline.

### Pipeline Architecture

```
filesrc -> decodebin -> videoconvert -> videoscale -> appsink
            (hw accel)      (RGBA)      (resize)     (extract)
```

### Thread Safety

The `VideoSource` uses `Arc<Mutex<>>` for thread-safe frame access. The GStreamer pipeline runs in separate threads managed by GStreamer's internal scheduler.

### Memory Management

- Frames are only kept in memory while being rendered
- Pipeline resources are released in the `Drop` implementation
- No frame buffering beyond GStreamer's internal queues

## Future Enhancements

Possible future improvements:

- [ ] Audio output support (currently muted)
- [ ] Playlist support for multiple videos
- [ ] Video effects/filters
- [ ] Per-output video configurations
- [ ] Subtitle rendering
- [ ] Power management integration (pause on idle)
- [ ] Frame rate limiting
- [ ] Multiple video layer compositing

## References

- [GStreamer Documentation](https://gstreamer.freedesktop.org/documentation/)
- [gstreamer-rs Bindings](https://gitlab.freedesktop.org/gstreamer/gstreamer-rs)
- [VA-API](https://github.com/intel/libva)
- [NVDEC](https://developer.nvidia.com/nvidia-video-codec-sdk)
