// SPDX-License-Identifier: MPL-2.0

//! GPU shader-based procedural wallpaper support using wgpu.
//!
//! This module provides real-time GPU-rendered animated backgrounds
//! using WGSL shaders. Includes built-in presets and custom shader support.

use crate::source::{Frame, SourceError, WallpaperSource};
use cosmic_bg_config::{ShaderConfig, ShaderPreset};
use image::{DynamicImage, ImageBuffer, Rgba};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Built-in shader source code
mod presets {
    pub const PLASMA: &str = include_str!("shaders/plasma.wgsl");
    pub const WAVES: &str = include_str!("shaders/waves.wgsl");
    pub const GRADIENT: &str = include_str!("shaders/gradient.wgsl");
}

/// Uniform buffer data for shaders
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    _padding: f32,
}

/// GPU shader wallpaper source
pub struct ShaderSource {
    config: ShaderConfig,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    output_texture: Option<wgpu::Texture>,
    output_buffer: Option<wgpu::Buffer>,
    target_size: Option<(u32, u32)>,
    start_time: Instant,
    last_frame: Instant,
    shader_source: String,
    is_prepared: bool,
}

impl std::fmt::Debug for ShaderSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaderSource")
            .field("config", &self.config)
            .field("target_size", &self.target_size)
            .field("is_prepared", &self.is_prepared)
            .finish_non_exhaustive()
    }
}

impl ShaderSource {
    /// Create a new shader source from configuration
    pub fn new(config: ShaderConfig) -> Result<Self, SourceError> {
        // Determine shader source code
        let shader_source = if let Some(ref path) = config.custom_path {
            Self::load_custom_shader(path)?
        } else if let Some(preset) = &config.preset {
            Self::get_preset_shader(preset)
        } else {
            // Default to gradient
            presets::GRADIENT.to_string()
        };

        Ok(Self {
            config,
            device: None,
            queue: None,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            output_texture: None,
            output_buffer: None,
            target_size: None,
            start_time: Instant::now(),
            last_frame: Instant::now(),
            shader_source,
            is_prepared: false,
        })
    }

    /// Load a custom shader from a file path
    fn load_custom_shader(path: &Path) -> Result<String, SourceError> {
        std::fs::read_to_string(path).map_err(|e| {
            SourceError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Failed to load custom shader: {}", e),
            ))
        })
    }

    /// Get the shader source for a preset
    fn get_preset_shader(preset: &ShaderPreset) -> String {
        match preset {
            ShaderPreset::Plasma => presets::PLASMA.to_string(),
            ShaderPreset::Waves => presets::WAVES.to_string(),
            ShaderPreset::Gradient => presets::GRADIENT.to_string(),
        }
    }

    /// Initialize GPU resources
    fn init_gpu(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        // Create wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Request adapter (prefer low-power for battery efficiency)
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| {
            SourceError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No suitable GPU adapter found",
            ))
        })?;

        tracing::info!(
            adapter = ?adapter.get_info().name,
            backend = ?adapter.get_info().backend,
            "GPU adapter selected for shader wallpaper"
        );

        // Create device and queue
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("cosmic-bg shader device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| {
            SourceError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create GPU device: {}", e),
            ))
        })?;

        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cosmic-bg shader"),
            source: wgpu::ShaderSource::Wgsl(self.shader_source.as_str().into()),
        });

        // Create uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
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

        // Create output texture
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
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // Create output buffer for CPU readback
        let bytes_per_row = Self::aligned_bytes_per_row(width);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output buffer"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        self.device = Some(device);
        self.queue = Some(queue);
        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
        self.output_texture = Some(output_texture);
        self.output_buffer = Some(output_buffer);
        self.target_size = Some((width, height));

        tracing::debug!(width, height, "Shader GPU resources initialized");

        Ok(())
    }

    /// Calculate aligned bytes per row (wgpu requires 256-byte alignment)
    fn aligned_bytes_per_row(width: u32) -> u32 {
        let unaligned = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        (unaligned + align - 1) / align * align
    }

    /// Render a frame and return the image
    fn render_frame(&mut self) -> Result<DynamicImage, SourceError> {
        let device = self.device.as_ref().ok_or_else(|| {
            SourceError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "GPU device not initialized",
            ))
        })?;
        let queue = self.queue.as_ref().unwrap();
        let pipeline = self.pipeline.as_ref().unwrap();
        let uniform_buffer = self.uniform_buffer.as_ref().unwrap();
        let bind_group = self.bind_group.as_ref().unwrap();
        let output_texture = self.output_texture.as_ref().unwrap();
        let output_buffer = self.output_buffer.as_ref().unwrap();
        let (width, height) = self.target_size.unwrap();

        // Update uniforms
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let uniforms = Uniforms {
            resolution: [width as f32, height as f32],
            time: elapsed,
            _padding: 0.0,
        };
        queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        // Create texture view
        let view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create command encoder
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render encoder"),
        });

        // Render pass
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
            render_pass.draw(0..3, 0..1); // Fullscreen triangle
        }

        // Copy texture to buffer
        let bytes_per_row = Self::aligned_bytes_per_row(width);
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
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(std::iter::once(encoder.finish()));

        // Read back the buffer
        let buffer_slice = output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });

        device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .unwrap()
            .map_err(|e| {
                SourceError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Buffer map failed: {}", e),
                ))
            })?;

        let data = buffer_slice.get_mapped_range();

        // Copy data to image (accounting for row alignment)
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }

        drop(data);
        output_buffer.unmap();

        // Create image from pixels
        let img_buffer: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(width, height, pixels).ok_or_else(|| {
                SourceError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Failed to create image buffer",
                ))
            })?;

        Ok(DynamicImage::ImageRgba8(img_buffer))
    }
}

impl WallpaperSource for ShaderSource {
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

    fn frame_duration(&self) -> Duration {
        let fps = self.config.fps_limit.max(1);
        let millis_per_frame = 1000u64 / fps as u64;
        Duration::from_millis(millis_per_frame)
    }

    fn is_animated(&self) -> bool {
        true
    }

    fn prepare(&mut self, width: u32, height: u32) -> Result<(), SourceError> {
        // Reinitialize if size changed
        if self.target_size != Some((width, height)) || self.device.is_none() {
            self.init_gpu(width, height)?;
        }

        self.is_prepared = true;
        Ok(())
    }

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

    fn description(&self) -> String {
        let preset_name = self
            .config
            .preset
            .as_ref()
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| "Custom".to_string());

        format!(
            "Shader: {} ({}fps)",
            preset_name,
            self.config.fps_limit
        )
    }
}

impl Drop for ShaderSource {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shader_config_defaults() {
        let config = ShaderConfig {
            preset: Some(ShaderPreset::Plasma),
            custom_path: None,
            fps_limit: 30,
        };

        let source = ShaderSource::new(config);
        assert!(source.is_ok());

        let source = source.unwrap();
        assert!(!source.is_prepared);
        assert!(source.is_animated());
        assert_eq!(source.frame_duration(), Duration::from_millis(33));
    }

    #[test]
    fn test_shader_description() {
        let config = ShaderConfig {
            preset: Some(ShaderPreset::Waves),
            custom_path: None,
            fps_limit: 60,
        };

        let source = ShaderSource::new(config).unwrap();
        let desc = source.description();

        assert!(desc.contains("Waves"));
        assert!(desc.contains("60fps"));
    }

    #[test]
    fn test_aligned_bytes_per_row() {
        // 256-byte alignment
        assert_eq!(ShaderSource::aligned_bytes_per_row(64), 256);
        assert_eq!(ShaderSource::aligned_bytes_per_row(100), 512);
        assert_eq!(ShaderSource::aligned_bytes_per_row(1920), 7680);
    }
}
