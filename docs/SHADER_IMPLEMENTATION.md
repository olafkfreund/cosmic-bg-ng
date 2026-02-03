# GPU Shader Wallpaper Implementation

## Overview

cosmic-bg-ng supports GPU-accelerated procedural wallpapers using wgpu and WGSL shaders. This feature enables real-time animated backgrounds with minimal CPU overhead, rendering directly on the GPU and supporting both built-in presets and custom shader files.

**Key Benefits:**
- Low CPU usage (< 2% GPU utilization)
- Smooth animations at configurable frame rates
- Battery-friendly with LowPower GPU adapter selection
- Extensible via custom WGSL shaders
- No external video files required

## wgpu Architecture

The implementation uses the wgpu graphics API, which provides a safe, portable GPU abstraction layer compatible with Vulkan, Metal, DirectX 12, and WebGPU.

### Core Components

#### **wgpu::Instance**
Entry point for GPU access. Created with all available backends:

```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::all(),
    ..Default::default()
});
```

#### **wgpu::Adapter**
Represents a physical GPU device. cosmic-bg-ng requests a LowPower adapter for battery efficiency:

```rust
let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
    power_preference: wgpu::PowerPreference::LowPower,
    compatible_surface: None,
    force_fallback_adapter: false,
}).await?;
```

The selected adapter details are logged:
```
INFO adapter="Intel UHD Graphics 620" backend=Vulkan "GPU adapter selected for shader wallpaper"
```

#### **wgpu::Device & Queue**
- **Device**: Creates GPU resources (buffers, textures, pipelines)
- **Queue**: Submits command buffers for execution

```rust
let (device, queue) = adapter.request_device(
    &wgpu::DeviceDescriptor {
        label: Some("cosmic-bg shader device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        memory_hints: wgpu::MemoryHints::Performance,
    },
    None,
).await?;
```

Uses downlevel limits for maximum compatibility with older GPUs.

#### **wgpu::RenderPipeline**
Compiled shader program ready for GPU execution. Defines:
- Vertex and fragment shader entry points
- Input/output formats
- Rendering state (blending, culling, etc.)

```rust
let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("shader pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
        module: &shader,
        entry_point: Some("vs_main"),
        buffers: &[],  // No vertex buffers (fullscreen triangle)
        compilation_options: Default::default(),
    },
    fragment: Some(wgpu::FragmentState {
        module: &shader,
        entry_point: Some("fs_main"),
        targets: &[Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
        compilation_options: Default::default(),
    }),
    primitive: wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        ..Default::default()
    },
    depth_stencil: None,
    multisample: wgpu::MultisampleState::default(),
    multiview: None,
    cache: None,
});
```

## ShaderSource Struct

The main implementation type that manages GPU resources and rendering.

### Field Breakdown

```rust
pub struct ShaderSource {
    config: ShaderConfig,                    // User configuration
    device: Option<wgpu::Device>,            // GPU device handle
    queue: Option<wgpu::Queue>,              // Command submission queue
    pipeline: Option<wgpu::RenderPipeline>,  // Compiled shader pipeline
    uniform_buffer: Option<wgpu::Buffer>,    // GPU buffer for uniforms
    bind_group: Option<wgpu::BindGroup>,     // Resource binding
    output_texture: Option<wgpu::Texture>,   // Render target texture
    output_buffer: Option<wgpu::Buffer>,     // CPU-readable buffer
    target_size: Option<(u32, u32)>,         // Current output dimensions
    start_time: Instant,                     // Animation start time
    last_frame: Instant,                     // Last frame render time
    shader_source: String,                   // WGSL source code
    is_prepared: bool,                       // Ready to render
}
```

**Design Notes:**
- Options allow lazy initialization and cleanup
- Resources recreated when output size changes
- `start_time` provides consistent time reference for animations
- `shader_source` loaded once at construction

### Resource Lifecycle

**Creation → Preparation → Rendering → Release**

1. **`new(config)`**: Loads shader source code
2. **`prepare(width, height)`**: Initializes GPU resources via `init_gpu()`
3. **`next_frame()`**: Renders frames on demand
4. **`release()`**: Drops all GPU resources (called by Drop trait)

## WallpaperSource Implementation

ShaderSource implements the `WallpaperSource` trait to integrate with cosmic-bg-ng's wallpaper system.

### prepare()

Initializes or reinitializes GPU resources when output size changes.

```rust
fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
    // Reinitialize if size changed
    if self.target_size != Some((width, height)) || self.device.is_none() {
        self.init_gpu(width, height)?;
    }

    self.is_prepared = true;
    Ok(())
}
```

**Triggers `init_gpu()` when:**
- First call (device is None)
- Output dimensions change (different monitor or resolution)

### next_frame()

Renders a single frame and returns it as a `DynamicImage`.

```rust
fn next_frame(&mut self) -> Result<Frame, SourceError> {
    if !self.is_prepared {
        return Err(SourceError::Io(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "Shader source not prepared",
        )));
    }

    let image = self.render_frame()?;
    self.last_frame = Instant::now();

    Ok(Frame {
        image,
        timestamp: Instant::now(),
    })
}
```

**Process:**
1. Validates prepared state
2. Calls `render_frame()` (see GPU Rendering Flow below)
3. Updates timestamp
4. Returns image wrapped in Frame

### frame_duration()

Calculates frame interval based on configured FPS limit.

```rust
fn frame_duration(&self) -> Duration {
    let fps = self.config.fps_limit.max(1);
    let millis_per_frame = 1000u64 / fps as u64;
    Duration::from_millis(millis_per_frame)
}
```

**Examples:**
- 30 FPS → 33ms per frame
- 60 FPS → 16ms per frame
- 15 FPS → 66ms per frame

### is_animated()

Always returns `true` for shaders since they're time-based.

```rust
fn is_animated(&self) -> bool {
    true
}
```

### release()

Drops all GPU resources to free memory.

```rust
fn release(&mut self) {
    self.device = None;
    self.queue = None;
    self.pipeline = None;
    self.uniform_buffer = None;
    self.bind_group = None;
    self.output_texture = None;
    self.output_buffer = None;
    self.is_prepared = false;

    tracing::debug!("Shader source released");
}
```

Called automatically when ShaderSource is dropped.

### description()

Returns human-readable source description for logging.

```rust
fn description(&self) -> String {
    let preset_name = self
        .config
        .preset
        .as_ref()
        .map(|p| format!("{:?}", p))
        .unwrap_or_else(|| "Custom".to_string());

    format!("Shader: {} ({}fps)", preset_name, self.config.fps_limit)
}
```

**Output examples:**
- `"Shader: Plasma (30fps)"`
- `"Shader: Custom (60fps)"`

## ShaderConfig

Configuration structure for shader wallpapers.

```rust
pub struct ShaderConfig {
    pub preset: Option<ShaderPreset>,
    pub custom_path: Option<PathBuf>,
    pub fps_limit: u32,
}
```

### Fields

- **`preset`**: Built-in shader preset (Plasma, Waves, Gradient)
- **`custom_path`**: Path to custom WGSL shader file
- **`fps_limit`**: Maximum frames per second (default: 30)

### Validation

Either `preset` or `custom_path` must be set, but not both:

```rust
impl ShaderConfig {
    pub fn is_valid(&self) -> bool {
        self.preset.is_some() != self.custom_path.is_some()
    }
}
```

### Default

```rust
impl Default for ShaderConfig {
    fn default() -> Self {
        Self {
            preset: Some(ShaderPreset::Plasma),
            custom_path: None,
            fps_limit: 30,
        }
    }
}
```

## Built-in Presets

### ShaderPreset Enum

```rust
pub enum ShaderPreset {
    Plasma,
    Waves,
    Gradient,
}
```

### Plasma

**File:** `src/shaders/plasma.wgsl`

Animated plasma effect using multiple overlapping sine waves.

**Visual Description:**
- Vibrant, morphing color patterns
- Multiple wave layers creating interference patterns
- Radial wave emanating from center
- Color cycles through full RGB spectrum
- Smooth, organic motion

**Technical Details:**
```wgsl
// Four wave layers
value += sin(uv.x * 10.0 + time);                    // Horizontal
value += sin(uv.y * 10.0 + time * 0.8);              // Vertical
value += sin((uv.x + uv.y) * 10.0 + time * 0.6);     // Diagonal
value += sin(dist * 20.0 - time * 2.0);              // Radial

// RGB color generation from wave value
let r = sin(value * 3.14159 * 2.0) * 0.5 + 0.5;
let g = sin(value * 3.14159 * 2.0 + 2.094) * 0.5 + 0.5;
let b = sin(value * 3.14159 * 2.0 + 4.188) * 0.5 + 0.5;
```

### Waves

**File:** `src/shaders/waves.wgsl`

Ocean-like flowing wave patterns with HSV color space.

**Visual Description:**
- Horizontal wave patterns with vertical influence
- Deep blue to cyan color range
- Multiple wave frequencies layered
- Smooth, flowing motion like water
- Saturation and value vary with wave height

**Technical Details:**
```wgsl
// Multi-layer wave composition
wave += sin(uv.y * 20.0 + time * 1.5) * 0.3;           // Primary wave
wave += sin(uv.y * 15.0 - time * 1.2 + uv.x * 5.0) * 0.2;  // Secondary
wave += sin(uv.y * 8.0 + time * 0.8 + uv.x * 3.0) * 0.15;  // Tertiary
wave += sin(uv.x * 12.0 + time * 0.5) * 0.1;          // Vertical

// HSV color mapping
let hue = 200.0 + wave * 60.0 + sin(time * 0.3) * 20.0;  // Blue range
let saturation = 0.6 + wave * 0.3;
let value = 0.3 + wave * 0.5;
```

Includes `hsv_to_rgb()` helper function for color space conversion.

### Gradient

**File:** `src/shaders/gradient.wgsl`

Rotating gradient with COSMIC-inspired purple/blue palette.

**Visual Description:**
- Smooth color transitions between 4 color stops
- Gradient angle rotates over time
- Deep purple to dark blue color scheme
- Subtle shimmer overlay effect
- Elegant, professional appearance

**Technical Details:**
```wgsl
// Gradient rotation
let angle = time * 0.2;
let rotated = vec2<f32>(
    centered.x * cos_a - centered.y * sin_a,
    centered.x * sin_a + centered.y * cos_a
);

// COSMIC color palette
let color1 = vec3<f32>(0.15, 0.1, 0.25);   // Deep purple
let color2 = vec3<f32>(0.25, 0.15, 0.4);   // Purple
let color3 = vec3<f32>(0.2, 0.3, 0.5);     // Blue-purple
let color4 = vec3<f32>(0.1, 0.2, 0.35);    // Dark blue

// Smooth interpolation with animation
let t = (sin(pos * 3.14159 + phase) + 1.0) * 0.5;

// Shimmer effect
let shimmer = sin(pos * 50.0 + time * 2.0) * 0.02 + 1.0;
```

## Uniforms Struct

GPU uniform buffer data passed to shaders each frame.

### Structure Definition

```rust
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],  // Width and height in pixels
    time: f32,             // Elapsed seconds since start
    _padding: f32,         // Alignment padding
}
```

**Attributes:**
- **`#[repr(C)]`**: C memory layout for GPU compatibility
- **`bytemuck::Pod`**: Plain Old Data - safe for byte casting
- **`bytemuck::Zeroable`**: Can be safely zeroed

### WGSL Declaration

Shaders access uniforms via binding 0:

```wgsl
struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;
```

### Usage in Shaders

**Resolution:**
```wgsl
// Convert fragment coordinates to normalized 0-1 range
let uv = frag_coord.xy / uniforms.resolution;
```

**Time:**
```wgsl
// Animate wave movement
let wave = sin(uv.x * 10.0 + uniforms.time);

// Rotate gradient
let angle = uniforms.time * 0.2;
```

### Update Process

Uniforms updated before each frame render:

```rust
let elapsed = self.start_time.elapsed().as_secs_f32();
let uniforms = Uniforms {
    resolution: [width as f32, height as f32],
    time: elapsed,
    _padding: 0.0,
};
queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
```

## GPU Buffer Management

### Texture Creation

Output texture for rendering results:

```rust
let output_texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("output texture"),
    size: wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::Rgba8Unorm,  // 8-bit RGBA
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    view_formats: &[],
});
```

**Usage flags:**
- `RENDER_ATTACHMENT`: Can be rendered to
- `COPY_SRC`: Can be copied to buffer for CPU readback

### Row Alignment

wgpu requires buffer row alignment for GPU efficiency:

```rust
fn aligned_bytes_per_row(width: u32) -> u32 {
    let unaligned = width * 4;  // 4 bytes per RGBA pixel
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;  // 256 bytes
    (unaligned + align - 1) / align * align
}
```

**Examples:**
- 1920px width → 7680 bytes (no padding needed, already aligned)
- 100px width → 512 bytes (padded from 400 to 512)
- 64px width → 256 bytes (exactly aligned)

### Buffer Readback

Output buffer for copying GPU data to CPU:

```rust
let bytes_per_row = Self::aligned_bytes_per_row(width);
let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("output buffer"),
    size: (bytes_per_row * height) as u64,
    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    mapped_at_creation: false,
});
```

**Usage flags:**
- `COPY_DST`: Texture can be copied into it
- `MAP_READ`: CPU can read the data

### GPU Rendering Flow

Complete render-to-image process:

```rust
fn render_frame(&mut self) -> Result<DynamicImage, SourceError> {
    // 1. Update uniforms with current time
    let elapsed = self.start_time.elapsed().as_secs_f32();
    let uniforms = Uniforms {
        resolution: [width as f32, height as f32],
        time: elapsed,
        _padding: 0.0,
    };
    queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

    // 2. Create render pass
    let view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("render encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shader render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);  // Draw fullscreen triangle
    }

    // 3. Copy texture to buffer
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: output_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: output_buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );

    queue.submit(std::iter::once(encoder.finish()));

    // 4. Map buffer and read pixels
    let buffer_slice = output_buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });

    device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap()?;

    let data = buffer_slice.get_mapped_range();

    // 5. Copy to image (handle row padding)
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * bytes_per_row) as usize;
        let end = start + (width * 4) as usize;
        pixels.extend_from_slice(&data[start..end]);
    }

    drop(data);
    output_buffer.unmap();

    // 6. Create DynamicImage
    let img_buffer: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(width, height, pixels).ok_or_else(|| {
            SourceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to create image buffer",
            ))
        })?;

    Ok(DynamicImage::ImageRgba8(img_buffer))
}
```

**Key Steps:**
1. Update time uniform
2. Render shader to texture
3. Copy texture to CPU-readable buffer
4. Map buffer memory to CPU
5. Strip row padding and create image
6. Unmap buffer and return image

## Writing Custom Shaders

Custom WGSL shaders must follow this template structure:

### Complete WGSL Template

```wgsl
// SPDX-License-Identifier: MPL-2.0
// My custom shader for cosmic-bg

// 1. Declare uniform buffer structure
struct Uniforms {
    resolution: vec2<f32>,  // Screen dimensions in pixels
    time: f32,              // Seconds since shader started
    _padding: f32,          // Required for alignment
}

// 2. Bind uniform buffer
@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

// 3. Vertex shader (DO NOT MODIFY - fullscreen triangle technique)
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vertex_index) - 1);
    let y = f32(i32(vertex_index & 1u) * 2 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// 4. Fragment shader (YOUR CODE HERE)
@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    // Normalize coordinates to 0-1 range
    let uv = frag_coord.xy / uniforms.resolution;

    // Access animation time
    let time = uniforms.time;

    // YOUR SHADER LOGIC HERE
    // Example: Simple animated color
    let r = (sin(uv.x * 10.0 + time) + 1.0) * 0.5;
    let g = (sin(uv.y * 10.0 + time * 0.8) + 1.0) * 0.5;
    let b = (sin((uv.x + uv.y) * 10.0 + time * 0.6) + 1.0) * 0.5;

    let color = vec3<f32>(r, g, b);

    // Return final color (alpha always 1.0)
    return vec4<f32>(color, 1.0);
}
```

### Required Elements

1. **Uniforms struct**: Must match Rust definition exactly
2. **Uniform binding**: `@group(0) @binding(0)`
3. **Vertex shader**: Named `vs_main`, uses fullscreen triangle
4. **Fragment shader**: Named `fs_main`, outputs RGBA color

### Available Inputs

**Fragment shader receives:**
- `frag_coord`: Pixel position (0,0 = top-left corner)
- `uniforms.resolution`: Screen size in pixels
- `uniforms.time`: Elapsed seconds (f32)

### Best Practices

**Coordinate Normalization:**
```wgsl
// Always normalize coordinates first
let uv = frag_coord.xy / uniforms.resolution;
// Now uv.x and uv.y are in 0-1 range
```

**Center-based Coordinates:**
```wgsl
// Center at (0,0) for radial effects
let centered = uv - vec2<f32>(0.5, 0.5);
let dist = length(centered);
```

**Animation:**
```wgsl
// Use time for smooth animation
let wave = sin(uv.x * frequency + uniforms.time * speed);

// Rotate over time
let angle = uniforms.time * rotation_speed;
let rotated = vec2<f32>(
    centered.x * cos(angle) - centered.y * sin(angle),
    centered.x * sin(angle) + centered.y * cos(angle)
);
```

**Color Generation:**
```wgsl
// Sine waves for smooth color cycles
let r = (sin(value * 3.14159 * 2.0) + 1.0) * 0.5;
let g = (sin(value * 3.14159 * 2.0 + 2.094) + 1.0) * 0.5;
let b = (sin(value * 3.14159 * 2.0 + 4.188) + 1.0) * 0.5;
```

### Common Patterns

**Distance-based Effects:**
```wgsl
let centered = uv - vec2<f32>(0.5, 0.5);
let dist = length(centered);
let circle = smoothstep(0.4, 0.5, dist);
```

**Layered Waves:**
```wgsl
var value = 0.0;
value += sin(uv.x * 10.0 + time) * 0.5;
value += sin(uv.y * 8.0 - time * 0.8) * 0.3;
value += sin((uv.x + uv.y) * 6.0 + time * 0.5) * 0.2;
```

**Gradient Rotation:**
```wgsl
let angle = time * 0.2;
let cos_a = cos(angle);
let sin_a = sin(angle);
let rotated_x = centered.x * cos_a - centered.y * sin_a;
```

### Saving and Loading

1. Save shader to file (e.g., `/home/user/.config/cosmic-bg/my_shader.wgsl`)
2. Configure in cosmic-bg-config:

```ron
(
    output: "all",
    source: Shader((
        preset: None,
        custom_path: Some("/home/user/.config/cosmic-bg/my_shader.wgsl"),
        fps_limit: 30,
    )),
    // ... other fields
)
```

## Performance

### FPS Limiting

Frame rate controlled by `fps_limit` configuration:

```rust
fn frame_duration(&self) -> Duration {
    let fps = self.config.fps_limit.max(1);
    let millis_per_frame = 1000u64 / fps as u64;
    Duration::from_millis(millis_per_frame)
}
```

**Common FPS values:**
- **15 FPS**: Maximum battery saving (66ms per frame)
- **30 FPS**: Balanced performance (33ms per frame) - **DEFAULT**
- **60 FPS**: Smooth animation (16ms per frame)

### GPU Resource Usage

**Typical resource consumption:**
- GPU utilization: < 2%
- VRAM usage: Output texture + buffer (~8MB for 1920x1080)
- Power preference: LowPower adapter for battery efficiency

**Resource allocation per output:**
```
Texture:  width × height × 4 bytes (RGBA)
Buffer:   aligned_row_size × height

Example (1920×1080):
Texture:  1920 × 1080 × 4 = 8,294,400 bytes (~8 MB)
Buffer:   7680 × 1080 = 8,294,400 bytes (~8 MB)
Total:    ~16 MB per output
```

### Memory Management

Resources automatically cleaned up:
- `Drop` trait calls `release()` when ShaderSource destroyed
- GPU resources recreated when output size changes
- No memory leaks from orphaned GPU objects

### Optimization Recommendations

**For battery-powered devices:**
```ron
ShaderConfig(
    preset: Some(Gradient),  // Simple shader
    fps_limit: 15,           // Low frame rate
)
```

**For desktop workstations:**
```ron
ShaderConfig(
    preset: Some(Plasma),    // Complex shader
    fps_limit: 60,           // High frame rate
)
```

**Shader complexity considerations:**
- Minimize per-pixel calculations
- Use lookup tables for complex functions
- Avoid branches in fragment shaders
- Leverage GPU's parallel processing

## Integration with cosmic-bg-ng

### Source Enum Integration

Shader support added to config Source enum:

```rust
pub enum Source {
    Path(PathBuf),          // Static image or slideshow directory
    Color(Color),           // Solid color or gradient
    Shader(ShaderConfig),   // GPU shader (NEW)
}
```

### Wallpaper Flow

**Configuration → Source Creation → Rendering:**

1. User configures `Source::Shader(config)` via cosmic-config
2. `Wallpaper` struct detects shader source
3. Creates `ShaderSource::new(config)`
4. Calls `prepare(width, height)` when output configured
5. Periodically calls `next_frame()` based on `frame_duration()`
6. Converts frame to XRGB8888 via `draw.rs`
7. Attaches to wl_shm buffer on layer surface

### Example Integration (Planned)

```rust
// In wallpaper.rs
match &entry.source {
    Source::Path(path) => {
        // Existing image source handling
    }
    Source::Color(color) => {
        // Existing color source handling
    }
    Source::Shader(shader_config) => {
        let mut source = ShaderSource::new(shader_config.clone())?;
        source.prepare(width, height)?;

        loop {
            let frame = source.next_frame()?;
            let duration = source.frame_duration();

            // Convert and display frame
            draw::write_to_buffer(frame.image, buffer, viewport);

            // Wait before next frame
            std::thread::sleep(duration);
        }
    }
}
```

## Configuration Examples

### Using Built-in Preset

```ron
(
    output: "all",
    source: Shader((
        preset: Some(Plasma),
        custom_path: None,
        fps_limit: 30,
    )),
    filter_by_theme: false,
    rotation_frequency: 0,
    filter_method: Lanczos,
    scaling_mode: Zoom,
    sampling_method: Alphanumeric,
)
```

### Custom Shader Per-Output

```ron
// Main display: High FPS waves
(
    output: "DP-1",
    source: Shader((
        preset: Some(Waves),
        custom_path: None,
        fps_limit: 60,
    )),
    // ... other fields
)

// Laptop display: Battery-efficient gradient
(
    output: "eDP-1",
    source: Shader((
        preset: Some(Gradient),
        custom_path: None,
        fps_limit: 15,
    )),
    // ... other fields
)
```

### Custom Shader File

```ron
(
    output: "HDMI-A-1",
    source: Shader((
        preset: None,
        custom_path: Some("/home/user/.config/cosmic-bg/shaders/starfield.wgsl"),
        fps_limit: 45,
    )),
    // ... other fields
)
```

## Future Enhancements

### Additional Uniform Parameters

Planned expansion of uniform buffer:

```rust
struct ExtendedUniforms {
    resolution: [f32; 2],
    time: f32,
    _padding: f32,
    // NEW:
    mouse_pos: [f32; 2],     // Mouse position (if interactive)
    system_time: f32,        // System time of day
    battery_level: f32,      // Battery percentage
    user_params: [f32; 4],   // Custom user parameters
}
```

### Hot Reload

Watch custom shader files for changes:

```rust
impl ShaderSource {
    fn watch_file(&mut self) -> notify::Result<()> {
        if let Some(path) = &self.config.custom_path {
            let watcher = notify::watcher(tx, Duration::from_secs(1))?;
            watcher.watch(path, RecursiveMode::NonRecursive)?;
            // Reload shader on file change
        }
        Ok(())
    }
}
```

### HDR Support

Add high dynamic range output:

```rust
format: wgpu::TextureFormat::Rgba16Float,  // 16-bit float HDR
```

### Compute Shaders

More complex effects via compute pipelines:

```rust
let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
    label: Some("particle system"),
    layout: Some(&pipeline_layout),
    module: &shader,
    entry_point: "compute_particles",
    compilation_options: Default::default(),
    cache: None,
});
```

### Multi-pass Rendering

Post-processing effects:

```rust
// Pass 1: Render scene
render_pass.draw(0..3, 0..1);

// Pass 2: Apply bloom
let bloom_pass = encoder.begin_render_pass(...);
bloom_pass.draw(0..3, 0..1);

// Pass 3: Tonemap to output
let final_pass = encoder.begin_render_pass(...);
final_pass.draw(0..3, 0..1);
```

## Troubleshooting

### GPU Not Found

**Error:** `"No suitable GPU adapter found"`

**Solutions:**
- Ensure GPU drivers installed
- Check Vulkan/Metal/DX12 support
- Try fallback adapter: `force_fallback_adapter: true`

### Shader Compilation Failed

**Error:** Shader validation errors

**Check:**
- Uniforms struct matches exactly
- Entry points named `vs_main` and `fs_main`
- All functions return correct types
- No missing semicolons or syntax errors

### Poor Performance

**Symptoms:** High GPU usage, frame drops

**Optimize:**
- Reduce `fps_limit` value
- Simplify shader calculations
- Use simpler preset (Gradient < Waves < Plasma)
- Check for expensive per-pixel operations

### Black Screen

**Possible causes:**
- Shader returns vec4(0.0, 0.0, 0.0, 1.0) (black)
- Division by zero in shader
- NaN values from invalid math operations

**Debug:**
```wgsl
// Output UV coordinates as colors
return vec4<f32>(uv.x, uv.y, 0.0, 1.0);
```

## References

- **Issue #8**: Original feature request for GPU shader support
- **wgpu documentation**: https://wgpu.rs/
- **WGSL specification**: https://www.w3.org/TR/WGSL/
- **cosmic-bg-config**: Configuration types and validation

## License

All shader code licensed under MPL-2.0. All source files must include:

```
// SPDX-License-Identifier: MPL-2.0
```
