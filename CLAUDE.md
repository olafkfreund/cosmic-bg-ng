# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

cosmic-bg is a Wayland background service for the COSMIC Desktop Environment (System76). It renders wallpapers to display outputs using wlr-layer-shell protocol surfaces. The service supports static images, colors, gradients, per-display backgrounds, and slideshow rotation.

## Build Commands

```bash
# Build release (default)
just

# Build debug
just build-debug

# Run with debug logging (kill cosmic-bg first to prevent cosmic-session respawning it)
just run

# Lint with pedantic clippy
just check

# Install (after release build)
sudo just install

# Install with custom prefix (for packaging)
just rootdir=debian/cosmic-bg prefix=/usr install
```

## Nix Development

```bash
nix build              # Build package
nix develop            # Enter dev shell with all dependencies
```

## Architecture

### Workspace Structure

- **`cosmic-bg`** (main crate): Wayland client service using smithay-client-toolkit
- **`cosmic-bg-config`** (config crate): Configuration types and cosmic-config integration

### Core Components

**main.rs**: Event loop setup with calloop, Wayland protocol handlers (compositor, output, layer-shell, shm), and cosmic-config watching. Key types:
- `CosmicBg`: Main state holding wallpapers, Wayland state, and config
- `CosmicBgLayer`: Per-output layer surface with viewport and scaling info

**wallpaper.rs**: `Wallpaper` struct manages background entries, image queues for slideshows, timer-based rotation, and filesystem watching via notify. Handles state persistence to resume slideshows after restart.

**draw.rs**: Buffer management via wl_shm slot pools. Converts images to XRGB8888 (8-bit) or XRGB2101010 (10-bit) format for surface attachment.

**scaler.rs**: Image scaling using fast_image_resize with Lanczos3. Implements Fit (letterbox with color), Zoom (crop to fill), and Stretch modes.

**colored.rs**: Generates solid color and gradient backgrounds using colorgrad.

**img_source.rs**: Filesystem event channel that updates wallpaper queues when images are added/removed from watched directories.

### Config Crate (config/)

**lib.rs**: Configuration types for cosmic-config integration:
- `Entry`: Per-output background configuration (source, scaling, rotation frequency, sampling method)
- `Source`: Path or Color (single/gradient)
- `Config`: Runtime configuration with backgrounds list and defaults
- `Context`: cosmic-config wrapper for reading/writing settings

**state.rs**: Persists current wallpaper state to resume slideshows across restarts.

### Data Flow

1. cosmic-config provides background configuration per output or "all"
2. Config watcher triggers `apply_backgrounds()` on changes
3. Each `Wallpaper` creates layer surfaces for matched outputs
4. On configure events, images are scaled and drawn to wl_shm buffers
5. Timer events rotate slideshow images; filesystem events update queues

### Key Dependencies

- **smithay-client-toolkit (sctk)**: Wayland client abstractions
- **calloop**: Event loop with Wayland source integration
- **cosmic-config**: COSMIC desktop configuration system with change notifications
- **fast_image_resize**: High-performance image scaling
- **jxl-oxide**: JPEG XL support
- **notify**: Filesystem watching for live wallpaper updates

## Configuration Path

Config stored via cosmic-config at `com.system76.CosmicBackground` (version 1).

## License

MPL-2.0. All source files require SPDX header: `// SPDX-License-Identifier: MPL-2.0`
