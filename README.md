# cosmic-bg

COSMIC session service which applies backgrounds to displays. A next-generation Wayland background daemon for the COSMIC Desktop Environment (System76) with support for static wallpapers, animated images, video playback, and GPU shader-based procedural backgrounds.

## Features

### Static Wallpapers
- **Image Formats**: Full support via [image-rs](https://github.com/image-rs/image#supported-image-formats) (JPEG, PNG, WebP, BMP, TIFF, etc.)
- **JPEG XL**: Native support via jxl-oxide for modern HDR images
- **Colors & Gradients**: Solid colors and multi-stop gradients using colorgrad
- **Slideshows**: Periodic wallpaper rotation with configurable intervals
- **Per-Display**: Independent backgrounds for each monitor

### Animated Wallpapers
- **GIF Support**: Full animation with per-frame timing
- **APNG Support**: Animated PNG with proper delay handling
- **Animated WebP**: WebP animation playback
- **FPS Limiting**: Configurable frame rate cap to reduce CPU usage
- **Loop Control**: Infinite or fixed loop count

### Video Wallpapers
- **Formats**: MP4, WebM, and other GStreamer-supported formats
- **Hardware Acceleration**: VA-API and NVDEC detection for efficient decoding
- **GStreamer Backend**: Full playback pipeline with appsink frame extraction
- **Loop Playback**: Seamless video looping
- **Power Aware**: Designed for minimal battery impact

### GPU Shader Wallpapers
- **wgpu Backend**: Cross-platform GPU compute using Vulkan/Metal/DX12
- **Built-in Presets**:
  - `Plasma` - Classic plasma effect with time-varying colors
  - `Waves` - Layered wave animation with HSV coloring
  - `Gradient` - Animated multi-stop gradient with rotation
- **Custom Shaders**: Load your own WGSL shaders
- **FPS Limiting**: Configurable frame rate for battery savings

### Performance & Architecture
- **Shared Image Cache**: Thread-safe LRU cache reduces memory when multiple outputs use the same wallpaper
- **Async Loading**: Background worker thread for non-blocking image decoding
- **Frame Scheduling**: Min-heap priority queue coordinates animation timing across outputs
- **Differential Updates**: Config changes only affect modified wallpapers, not full rebuild
- **HDR Support**: 10-bit (XRGB2101010) surface rendering for HDR displays
- **Transform Handling**: Proper dimension calculation for rotated displays (90/180/270)

### Error Handling
- **Structured Errors**: `thiserror`-based error types with proper propagation
- **Graceful Degradation**: Fallback to solid color on image load failures
- **Comprehensive Logging**: `tracing` instrumentation at info/debug/trace levels

## Module Reference

| Module | Description | Lines |
|--------|-------------|-------|
| `error.rs` | Error types with thiserror derive macros | ~65 |
| `source.rs` | `WallpaperSource` trait and `StaticSource`/`ColorSource` implementations | ~240 |
| `cache.rs` | Thread-safe LRU image cache with configurable limits | ~365 |
| `scheduler.rs` | Frame timing scheduler using BinaryHeap priority queue | ~295 |
| `loader.rs` | Async image loading with worker thread | ~340 |
| `animated.rs` | GIF/APNG/WebP animated image support | ~470 |
| `video.rs` | GStreamer video playback with hardware acceleration | ~495 |
| `shader.rs` | wgpu GPU shader rendering with compute pipeline | ~510 |

## Dependencies

### Build Dependencies

```bash
# Debian/Ubuntu
sudo apt install just mold pkg-config libwayland-dev libxkbcommon-dev

# For video wallpapers
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
                 gstreamer1.0-plugins-good gstreamer1.0-plugins-bad

# For shader wallpapers (runtime)
# GPU with Vulkan, Metal, or DX12 support
```

### Rust Version
Requires Rust 1.85+ (edition 2024). Install via https://rustup.rs/

### Nix Development
```bash
nix develop  # Enter dev shell with all dependencies
nix build    # Build the package
```

## Installation

```bash
# Build release
just

# Install system-wide
sudo just install

# Package installation (e.g., for Debian)
just rootdir=debian/cosmic-bg prefix=/usr install
```

## Configuration

Configuration is stored via cosmic-config at `com.system76.CosmicBackground` (version 1).

### Static Wallpaper
```ron
(
    output: "all",
    source: Path("/usr/share/backgrounds/cosmic.jpg"),
    scaling_mode: Zoom,
    rotation_frequency: 3600,  // Slideshow rotation in seconds
)
```

### Color or Gradient
```ron
(
    output: "all",
    source: Color(Single([0.2, 0.4, 0.8])),  // RGB 0.0-1.0
    scaling_mode: Zoom,
)

// Gradient
(
    output: "all",
    source: Color(Gradient {
        colors: [[0.1, 0.2, 0.5], [0.3, 0.1, 0.4]],
        radius: 0.5,
    }),
)
```

### GPU Shader
```ron
(
    output: "all",
    source: Shader(
        preset: Some(Plasma),  // Plasma, Waves, or Gradient
        custom_path: None,     // Or Some("/path/to/shader.wgsl")
        fps_limit: 30,
    ),
)
```

### Per-Display Configuration
```ron
// Different wallpaper per output
[
    (
        output: "DP-1",
        source: Path("/home/user/wallpapers/left.jpg"),
        scaling_mode: Zoom,
    ),
    (
        output: "HDMI-A-1",
        source: Shader(preset: Some(Waves), fps_limit: 60),
    ),
]
```

## Scaling Modes

| Mode | Description |
|------|-------------|
| `Fit` | Scale to fit within bounds, letterbox with background color |
| `Zoom` | Scale to fill, crop edges as needed |
| `Stretch` | Stretch to fill exactly (may distort) |

## Architecture

```
cosmic-bg/
├── src/
│   ├── main.rs          # Event loop, Wayland handlers, config watching
│   ├── wallpaper.rs     # Wallpaper state and rendering coordination
│   ├── draw.rs          # Buffer management, HDR format selection
│   ├── scaler.rs        # Image scaling with fast_image_resize
│   ├── colored.rs       # Solid colors and gradients via colorgrad
│   ├── img_source.rs    # Filesystem watching for directories
│   ├── error.rs         # Structured error types
│   ├── source.rs        # WallpaperSource trait system
│   ├── cache.rs         # LRU image cache
│   ├── scheduler.rs     # Frame timing infrastructure
│   ├── loader.rs        # Async image loading
│   ├── animated.rs      # Animated image support
│   ├── video.rs         # Video wallpaper support
│   ├── shader.rs        # GPU shader support
│   └── shaders/         # Built-in WGSL presets
│       ├── plasma.wgsl
│       ├── waves.wgsl
│       └── gradient.wgsl
├── config/
│   ├── lib.rs           # Configuration types (Entry, Source, ShaderConfig)
│   └── state.rs         # Persistent state for slideshow position
└── Cargo.toml
```

### Data Flow

```
cosmic-config ──> Config Watcher ──> apply_backgrounds()
                                            │
                                            ▼
                        ┌─────────────────────────────────────┐
                        │           Wallpaper                  │
                        │  ┌─────────────────────────────┐    │
                        │  │      WallpaperSource        │    │
                        │  │  ┌─────────────────────┐    │    │
                        │  │  │ Static │ Animated │ │    │    │
                        │  │  │ Video  │ Shader   │ │    │    │
                        │  │  └─────────────────────┘    │    │
                        │  └─────────────────────────────┘    │
                        │               │                      │
                        │               ▼                      │
                        │        ImageCache                    │
                        │               │                      │
                        │               ▼                      │
                        │     FrameScheduler                   │
                        └───────────────│─────────────────────┘
                                        │
                                        ▼
                              wl_shm Buffer ──> Layer Surface
```

## Debugging

```bash
# Kill existing instance (cosmic-session will respawn it)
pkill cosmic-bg

# Run with debug logging
RUST_LOG=cosmic_bg=debug just run

# Trace-level for frame timing details
RUST_LOG=cosmic_bg=trace just run

# Specific module debugging
RUST_LOG=cosmic_bg::cache=debug,cosmic_bg::shader=trace just run
```

## Writing Custom Shaders

Custom WGSL shaders receive these uniforms:

```wgsl
struct Uniforms {
    time: f32,           // Elapsed time in seconds
    resolution: vec2f,   // Output dimensions (width, height)
    mouse: vec2f,        // Reserved for future use
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var output_texture: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) global_id: vec3u) {
    let coords = vec2i(global_id.xy);
    let uv = vec2f(global_id.xy) / uniforms.resolution;

    // Your shader logic here
    let color = vec4f(uv.x, uv.y, sin(uniforms.time), 1.0);

    textureStore(output_texture, coords, color);
}
```

## Integration Status

| Feature | Implementation | Config Integration |
|---------|---------------|-------------------|
| Static Images | Complete | Complete |
| Colors/Gradients | Complete | Complete |
| GPU Shaders | Complete | Complete |
| Animated Images | Complete | Pending |
| Video Wallpapers | Complete | Pending |
| Image Cache | Complete | Auto-enabled |
| Async Loader | Complete | Pending |
| Frame Scheduler | Complete | Pending |

*"Pending" means the core implementation exists but config-level `Source::Animation` and `Source::Video` variants are not yet added to cosmic-bg-config.*

## License

Licensed under the [Mozilla Public License Version 2.0](https://choosealicense.com/licenses/mpl-2.0).

### Contribution

Any contribution intentionally submitted for inclusion in the work by you shall be licensed under the Mozilla Public License Version 2.0 (MPL-2.0). Each source file should have a SPDX copyright notice:

```
// SPDX-License-Identifier: MPL-2.0
```
