<!-- This page is seeded from README.md. Edit either file;
     they diverge by design after the initial onboarding. -->

# cosmic-ext-bg

[![Release](https://img.shields.io/github/v/release/olafkfreund/cosmic-ext-bg)](https://github.com/olafkfreund/cosmic-ext-bg/releases)
[![License: MPL-2.0](https://img.shields.io/badge/License-MPL--2.0-blue.svg)](https://choosealicense.com/licenses/mpl-2.0)
[![AUR](https://img.shields.io/badge/AUR-cosmic--ext--bg--git-blue)](https://aur.archlinux.org/packages/cosmic-ext-bg-git)

A next-generation Wayland background service for the COSMIC Desktop Environment (System76). Drop-in replacement for `cosmic-bg` with support for static wallpapers, animated images, video playback, and GPU shader-based procedural backgrounds.

## Features

### Static Wallpapers
- **Image Formats**: Full support via [image-rs](https://github.com/image-rs/image#supported-image-formats) (JPEG, PNG, WebP, BMP, TIFF, etc.)
- **JPEG XL**: Native support via jxl-oxide for modern HDR images
- **Colors & Gradients**: Solid colors and multi-stop gradients with cached trig computation
- **Slideshows**: Periodic wallpaper rotation with configurable intervals (skips single-image queues)
- **Per-Display**: Independent backgrounds for each monitor

### Animated Wallpapers
- **GIF Support**: Full animation with per-frame timing
- **APNG Support**: Animated PNG with proper delay handling
- **Animated WebP**: WebP animation playback
- **FPS Limiting**: Configurable frame rate cap to reduce CPU usage
- **Loop Control**: Infinite or fixed loop count
- **Memory Protection**: Frame count limited to 5,000 to prevent OOM

### Video Wallpapers
- **Formats**: MP4, WebM, and other GStreamer-supported formats
- **Hardware Acceleration**: VA-API and NVDEC detection for efficient decoding
- **GStreamer Backend**: Full playback pipeline with appsink frame extraction
- **Loop Playback**: Seamless video looping
- **Speed Control**: Adjustable playback speed (0.1x–10.0x, safely clamped)

### GPU Shader Wallpapers
- **wgpu Backend**: Cross-platform GPU compute using Vulkan/Metal/DX12
- **Built-in Presets**:
  - `Plasma` — Classic plasma effect with time-varying colors
  - `Waves` — Layered wave animation with HSV coloring
  - `Gradient` — Animated multi-stop gradient with rotation
- **Custom Shaders**: Load your own WGSL shaders (validated: 64 KB max, `.wgsl` extension)
- **FPS Limiting**: Configurable frame rate (1–240 FPS, safely clamped)

### Performance & Reliability
- **Shared Image Cache**: Thread-safe LRU cache reduces memory when multiple outputs use the same wallpaper
- **Async Loading**: Background worker thread for non-blocking image decoding
- **Frame Scheduling**: Min-heap priority queue coordinates animation timing across outputs
- **Differential Updates**: Config changes only affect modified wallpapers
- **HDR Support**: 10-bit (XRGB2101010) surface rendering for HDR displays
- **Buffer Overflow Protection**: Checked arithmetic for buffer size calculations
- **Safe Error Handling**: No `unwrap()` in video/shader paths; structured `SourceError` types with `thiserror`
- **Filesystem Watching**: Live directory monitoring with stored watcher lifetime management
- **Comprehensive Logging**: `tracing` instrumentation at info/debug/trace levels

## Installation

### Arch Linux (AUR)

```bash
# Install service + CLI tool (replaces cosmic-bg)
yay -S cosmic-ext-bg-git

# Optional: GUI settings application
yay -S cosmic-ext-bg-settings-git
```

### NixOS

```nix
# flake.nix
{
  inputs.cosmic-ext-bg.url = "github:olafkfreund/cosmic-ext-bg";
}

# configuration.nix
{ inputs, ... }: {
  imports = [ inputs.cosmic-ext-bg.nixosModules.default ];

  services.cosmic-ext-bg = {
    enable = true;
    replaceSystemPackage = true;  # Replaces cosmic-bg system-wide (default)

    settings = {
      enableVideoWallpapers = true;    # GStreamer video support
      enableShaderWallpapers = true;   # GPU shader support
      enableAnimatedWallpapers = true; # GIF/APNG/WebP support
    };
  };
}
```

Or build directly:

```bash
nix build                       # Build service + CLI
nix build .#cosmic-ext-bg-ctl     # CLI tool only
nix build .#cosmic-ext-bg-settings # GUI only
nix develop                     # Enter dev shell with all dependencies
```

### Debian/Ubuntu

```bash
sudo apt install just mold pkg-config libwayland-dev libxkbcommon-dev \
                 libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
                 gstreamer1.0-plugins-good gstreamer1.0-plugins-bad

just build-release
sudo just install
sudo just install-ctl
```

### From Source

Requires Rust 1.85+ (edition 2024). Install via https://rustup.rs/

```bash
# Build all binaries (service + CLI)
just

# Build everything including GUI
just build-all

# Install everything
sudo just install-all

# Package installation (custom prefix)
just rootdir=debian/cosmic-ext-bg prefix=/usr install
```

### Shell Completions

```bash
just completions

# Or manually
cosmic-ext-bg-ctl completions bash > ~/.local/share/bash-completion/completions/cosmic-ext-bg-ctl
cosmic-ext-bg-ctl completions zsh > ~/.zsh/completions/_cosmic-ext-bg-ctl
cosmic-ext-bg-ctl completions fish > ~/.config/fish/completions/cosmic-ext-bg-ctl.fish
```

## Tools

### cosmic-ext-bg-ctl (CLI)

Command-line tool for managing wallpapers without editing config files directly.

```bash
# Set a static wallpaper
cosmic-ext-bg-ctl set /path/to/image.png
cosmic-ext-bg-ctl set /path/to/wallpapers/ -r 300  # Slideshow, rotate every 5 min

# Set a video wallpaper
cosmic-ext-bg-ctl video /path/to/video.mp4 --loop --speed 1.5

# Set an animated wallpaper
cosmic-ext-bg-ctl animated /path/to/animation.gif --fps 30

# Set a GPU shader wallpaper
cosmic-ext-bg-ctl shader Plasma --fps 60
cosmic-ext-bg-ctl shader /path/to/custom.wgsl

# Set a solid color or gradient
cosmic-ext-bg-ctl color "#1a1b26"
cosmic-ext-bg-ctl color "#1a1b26" --gradient-colors "#24283b" "#414868" --radius 0.5

# Query current configuration
cosmic-ext-bg-ctl query
cosmic-ext-bg-ctl query -o DP-1

# List configured outputs
cosmic-ext-bg-ctl outputs

# Backup and restore
cosmic-ext-bg-ctl backup -f ~/my-wallpaper-config.ron
cosmic-ext-bg-ctl restore -f ~/my-wallpaper-config.ron

# Generate shell completions
cosmic-ext-bg-ctl completions bash
```

#### Commands Reference

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

#### Options

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

### cosmic-ext-bg-settings (GUI)

Graphical application for configuring wallpapers with live preview.

```bash
cosmic-ext-bg-settings
```

Features:
- Source type selector (Static, Video, Animated, Shader, Color, Gradient)
- XDG file picker dialog for selecting files
- Scaling mode dropdown
- Per-display configuration toggle
- Real-time config updates via cosmic-config

## Configuration

Configuration is stored via cosmic-config at `io.github.olafkfreund.CosmicExtBg` (version 1).

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

Shaders must be `.wgsl` files under 64 KB.

## Architecture

```
cosmic-ext-bg/
├── src/
│   ├── main.rs          # Event loop, Wayland handlers, config watching
│   ├── wallpaper.rs     # Wallpaper state and rendering coordination
│   ├── draw.rs          # Buffer management, HDR format selection
│   ├── scaler.rs        # Image scaling with fast_image_resize (Lanczos3)
│   ├── colored.rs       # Solid colors and gradients via colorgrad
│   ├── img_source.rs    # Filesystem watching for directories
│   ├── source.rs        # WallpaperSource trait, shared constants and errors
│   ├── cache.rs         # LRU image cache
│   ├── scheduler.rs     # Frame timing infrastructure
│   ├── loader.rs        # Async image loading
│   ├── animated.rs      # GIF/APNG/WebP animated image support
│   ├── video.rs         # GStreamer video wallpaper support
│   ├── shader.rs        # wgpu GPU shader support
│   ├── shaders/         # Built-in WGSL presets
│   │   ├── plasma.wgsl
│   │   ├── waves.wgsl
│   │   └── gradient.wgsl
│   └── bin/
│       └── cosmic-ext-bg-ctl.rs  # CLI tool
├── config/
│   ├── lib.rs           # Configuration types (Entry, Source, ShaderConfig, VideoConfig)
│   └── state.rs         # Persistent state for slideshow position
├── cosmic-ext-bg-settings/  # GUI application (libcosmic)
├── aur/                 # Arch Linux AUR packages
│   ├── cosmic-ext-bg-git/
│   └── cosmic-ext-bg-settings-git/
├── debian/              # Debian packaging
├── nix/
│   └── module.nix       # NixOS module
├── data/
│   ├── io.github.olafkfreund.CosmicExtBg.desktop
│   ├── io.github.olafkfreund.CosmicExtBgSettings.desktop
│   └── icons/
└── flake.nix            # Nix flake (crane + fenix)
```

### Data Flow

```
┌─────────────────┐     ┌──────────────────────┐
│ cosmic-ext-bg-ctl│     │ cosmic-ext-bg-settings│
│     (CLI)       │     │        (GUI)          │
└────────┬────────┘     └────────┬─────────────┘
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
pkill cosmic-ext-bg

# Run with debug logging
RUST_LOG=cosmic_ext_bg=debug just run

# Trace-level for frame timing details
RUST_LOG=cosmic_ext_bg=trace just run

# Specific module debugging
RUST_LOG=cosmic_ext_bg::shader=trace,cosmic_ext_bg::video=debug just run
```

## Integration Status

| Feature | Implementation | Config Integration | CLI | GUI |
|---------|---------------|-------------------|-----|-----|
| Static Images | Complete | Complete | Yes | Yes |
| Colors/Gradients | Complete | Complete | Yes | Yes |
| GPU Shaders | Complete | Complete | Yes | Yes |
| Animated Images | Complete | Complete | Yes | Yes |
| Video Wallpapers | Complete | Complete | Yes | Yes |
| Image Cache | Complete | Auto-enabled | — | — |
| Async Loader | Complete | Auto-enabled | — | — |
| Frame Scheduler | Complete | Auto-enabled | — | — |
| Shell Completions | Complete | — | Yes | — |
| XDG File Picker | Complete | — | — | Yes |

## Packaging

| Platform | Package | Status |
|----------|---------|--------|
| NixOS | Flake with NixOS module + overlay | Included |
| Arch Linux | `cosmic-ext-bg-git`, `cosmic-ext-bg-settings-git` (AUR) | Included |
| Debian/Ubuntu | `debian/` packaging | Included |

## License

Licensed under the [Mozilla Public License Version 2.0](https://choosealicense.com/licenses/mpl-2.0).

### Contribution

Any contribution intentionally submitted for inclusion in the work by you shall be licensed under the Mozilla Public License Version 2.0 (MPL-2.0). Each source file should have a SPDX copyright notice:

```
// SPDX-License-Identifier: MPL-2.0
```
