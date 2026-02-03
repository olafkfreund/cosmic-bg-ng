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

## Tools

### cosmic-bg-ctl (CLI)

Command-line tool for managing wallpapers without editing config files directly.

```bash
# Set a static wallpaper
cosmic-bg-ctl set /path/to/image.png
cosmic-bg-ctl set /path/to/wallpapers/ -r 300  # Slideshow, rotate every 5 min

# Set a video wallpaper
cosmic-bg-ctl video /path/to/video.mp4 --loop --speed 1.5

# Set an animated wallpaper
cosmic-bg-ctl animated /path/to/animation.gif --fps 30

# Set a GPU shader wallpaper
cosmic-bg-ctl shader Plasma --fps 60
cosmic-bg-ctl shader /path/to/custom.wgsl

# Set a solid color or gradient
cosmic-bg-ctl color "#1a1b26"
cosmic-bg-ctl color "#1a1b26" --gradient-colors "#24283b" "#414868" --radius 0.5

# Query current configuration
cosmic-bg-ctl query
cosmic-bg-ctl query -o DP-1

# List configured outputs
cosmic-bg-ctl outputs

# Backup and restore
cosmic-bg-ctl backup -f ~/my-wallpaper-config.ron
cosmic-bg-ctl restore -f ~/my-wallpaper-config.ron

# Generate shell completions
cosmic-bg-ctl completions bash > ~/.local/share/bash-completion/completions/cosmic-bg-ctl
cosmic-bg-ctl completions zsh > ~/.zsh/completions/_cosmic-bg-ctl
cosmic-bg-ctl completions fish > ~/.config/fish/completions/cosmic-bg-ctl.fish
```

#### CLI Commands Reference

| Command | Description |
|---------|-------------|
| `set <path>` | Set static image wallpaper (file or directory for slideshow) |
| `video <path>` | Set video wallpaper with loop/speed options |
| `animated <path>` | Set animated image wallpaper (GIF, WebP, APNG) |
| `shader <preset\|path>` | Set GPU shader (Plasma, Waves, Gradient, or custom .wgsl) |
| `color <hex>` | Set solid color or gradient wallpaper |
| `query` | Show current wallpaper configuration |
| `outputs` | List configured display outputs |
| `backup` | Save configuration to file |
| `restore` | Load configuration from file |
| `completions <shell>` | Generate shell completions (bash, zsh, fish) |

#### CLI Options

| Option | Commands | Description |
|--------|----------|-------------|
| `-o, --output` | all | Target specific display (e.g., DP-1, HDMI-A-1) |
| `-s, --scaling` | set | Scaling mode: zoom, fit, stretch |
| `-r, --rotation` | set | Slideshow rotation frequency in seconds |
| `--loop` | video | Enable loop playback |
| `--speed` | video | Playback speed multiplier |
| `--no-hw-accel` | video | Disable hardware acceleration |
| `--fps` | animated, shader | FPS limit |
| `--loops` | animated | Loop count (omit for infinite) |

### cosmic-bg-settings (GUI)

Graphical application for configuring wallpapers with live preview.

```bash
# Run the settings app
cosmic-bg-settings

# Or via justfile
just run-settings
```

Features:
- Source type selector (Static, Video, Animated, Shader, Color, Gradient)
- XDG file picker dialog for selecting files
- Scaling mode dropdown
- Per-display configuration toggle
- Real-time config updates via cosmic-config

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
# Build all binaries (service + CLI)
just

# Build everything including GUI
just build-all

# Install service only
sudo just install

# Install CLI tool
sudo just install-ctl

# Install settings GUI (with desktop entry)
sudo just install-settings

# Install everything
sudo just install-all

# Package installation (e.g., for Debian)
just rootdir=debian/cosmic-bg prefix=/usr install
```

### Nix Installation

```bash
# Build default package (service + CLI)
nix build

# Build specific package
nix build .#cosmic-bg-ctl
nix build .#cosmic-bg-settings

# Run directly
nix run .#cosmic-bg-ctl -- --help
nix run .#cosmic-bg-settings

# Add to NixOS configuration
{
  inputs.cosmic-bg-ng.url = "github:olafkfreund/cosmic-bg-ng";
}

# In configuration.nix
{ inputs, ... }: {
  imports = [ inputs.cosmic-bg-ng.nixosModules.default ];
  services.cosmic-bg.enable = true;
}
```

### Shell Completions

```bash
# Generate completions
just completions

# Or manually install
cosmic-bg-ctl completions bash > ~/.local/share/bash-completion/completions/cosmic-bg-ctl
cosmic-bg-ctl completions zsh > ~/.zsh/completions/_cosmic-bg-ctl
cosmic-bg-ctl completions fish > ~/.config/fish/completions/cosmic-bg-ctl.fish
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
cosmic-bg-ng/
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
│   ├── shaders/         # Built-in WGSL presets
│   │   ├── plasma.wgsl
│   │   ├── waves.wgsl
│   │   └── gradient.wgsl
│   └── bin/
│       └── cosmic-bg-ctl.rs  # CLI tool for wallpaper management
├── config/
│   ├── lib.rs           # Configuration types (Entry, Source, ShaderConfig)
│   └── state.rs         # Persistent state for slideshow position
├── cosmic-bg-settings/  # GUI application (libcosmic)
│   ├── src/
│   │   ├── main.rs      # Application entry point
│   │   ├── app.rs       # libcosmic Application impl
│   │   ├── message.rs   # UI message types
│   │   ├── config.rs    # Config helpers
│   │   ├── pages/       # UI pages
│   │   └── widgets/     # Custom widgets
│   └── i18n/            # Translations
├── data/
│   ├── com.system76.CosmicBackground.desktop
│   ├── com.system76.CosmicBgSettings.desktop
│   └── icons/
└── Cargo.toml
```

### Data Flow

```
┌─────────────────┐     ┌──────────────────┐
│ cosmic-bg-ctl   │     │ cosmic-bg-settings│
│     (CLI)       │     │      (GUI)        │
└────────┬────────┘     └────────┬─────────┘
         │                       │
         └───────────┬───────────┘
                     ▼
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

| Feature | Implementation | Config Integration | CLI | GUI |
|---------|---------------|-------------------|-----|-----|
| Static Images | Complete | Complete | ✓ | ✓ |
| Colors/Gradients | Complete | Complete | ✓ | ✓ |
| GPU Shaders | Complete | Complete | ✓ | ✓ |
| Animated Images | Complete | Complete | ✓ | ✓ |
| Video Wallpapers | Complete | Complete | ✓ | ✓ |
| Image Cache | Complete | Auto-enabled | - | - |
| Async Loader | Complete | Auto-enabled | - | - |
| Frame Scheduler | Complete | Auto-enabled | - | - |
| Shell Completions | Complete | - | ✓ | - |
| XDG File Picker | Complete | - | - | ✓ |

## License

Licensed under the [Mozilla Public License Version 2.0](https://choosealicense.com/licenses/mpl-2.0).

### Contribution

Any contribution intentionally submitted for inclusion in the work by you shall be licensed under the Mozilla Public License Version 2.0 (MPL-2.0). Each source file should have a SPDX copyright notice:

```
// SPDX-License-Identifier: MPL-2.0
```
