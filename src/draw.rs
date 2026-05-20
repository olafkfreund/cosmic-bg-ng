// SPDX-License-Identifier: MPL-2.0

use crate::{CosmicBg, CosmicBgLayer};
use image::{DynamicImage, GenericImageView};
use sctk::{
    reexports::client::{QueueHandle, protocol::wl_shm},
    shell::WaylandSurface,
    shm::slot::{Buffer, CreateBufferError, SlotPool},
};

pub fn canvas(
    pool: &mut SlotPool,
    image: &DynamicImage,
    width: i32,
    height: i32,
    stride: i32,
) -> Result<Buffer, CreateBufferError> {
    // TODO: Check if we need 8-bit or 10-bit
    let hdr_layer = false;

    let (buffer, canvas) = pool.create_buffer(
        width,
        height,
        stride,
        if hdr_layer {
            wl_shm::Format::Xrgb2101010
        } else {
            wl_shm::Format::Xrgb8888
        },
    )?;

    // Draw to the window:
    {
        if hdr_layer {
            xrgb21010_canvas(canvas, image);
        } else {
            xrgb888_canvas(canvas, image);
        }
    }

    Ok(buffer)
}

pub fn canvas_from_bgrx(
    pool: &mut SlotPool,
    data: &[u8],
    width: u32,
    height: u32,
    source_stride: usize,
) -> Result<Buffer, CreateBufferError> {
    let stride = width as i32 * 4;
    let (buffer, canvas) = pool.create_buffer(width as i32, height as i32, stride, wl_shm::Format::Xrgb8888)?;

    copy_bgrx_to_xrgb8888(canvas, data, width as usize, height as usize, source_stride, stride as usize);

    Ok(buffer)
}

pub fn copy_bgrx_to_xrgb8888(
    canvas: &mut [u8],
    data: &[u8],
    width: usize,
    height: usize,
    source_stride: usize,
    dest_stride: usize,
) {
    let row_bytes = width * 4;
    for row in 0..height {
        let src_start = row * source_stride;
        let src_end = src_start + row_bytes;
        let dst_start = row * dest_stride;
        let dst_end = dst_start + row_bytes;
        canvas[dst_start..dst_end].copy_from_slice(&data[src_start..src_end]);
    }
}

pub fn layer_surface(
    layer: &mut CosmicBgLayer,
    queue_handle: &QueueHandle<CosmicBg>,
    buffer: &Buffer,
    buffer_damage: (i32, i32),
) {
    let Some((width, height)) = layer.size else {
        tracing::error!("layer_surface called with no size configured");
        return;
    };

    let wl_surface = layer.layer.wl_surface();

    // Damage the entire window
    wl_surface.damage_buffer(0, 0, buffer_damage.0, buffer_damage.1);

    // Request our next frame
    layer
        .layer
        .wl_surface()
        .frame(queue_handle, wl_surface.clone());

    // Attach and commit to present.
    if let Err(why) = buffer.attach_to(wl_surface) {
        tracing::error!(?why, "buffer attachment failed");
    }

    layer.viewport.set_destination(width as i32, height as i32);

    wl_surface.commit();
}

/// Draws the image on a 10-bit canvas.
pub fn xrgb21010_canvas(canvas: &mut [u8], image: &DynamicImage) {
    debug_assert!(
        canvas.len() >= image.width() as usize * image.height() as usize * 4,
        "canvas too small: {} bytes for {}x{} image",
        canvas.len(), image.width(), image.height()
    );

    const BIT_MASK: u32 = (1 << 10) - 1;

    for (pos, pixel) in image.to_rgb16().pixels().enumerate() {
        let indice = pos * 4;

        let [r, g, b] = pixel.0;

        let r = ((u32::from(r) * BIT_MASK) & BIT_MASK) << 20;
        let g = ((u32::from(g) * BIT_MASK) & BIT_MASK) << 10;
        let b = (u32::from(b) * BIT_MASK) & BIT_MASK;

        canvas[indice..indice + 4].copy_from_slice(&(r | g | b).to_le_bytes());
    }
}

/// Draws the image on an 8-bit canvas.
pub fn xrgb888_canvas(canvas: &mut [u8], image: &DynamicImage) {
    debug_assert!(
        canvas.len() >= image.width() as usize * image.height() as usize * 4,
        "canvas too small: {} bytes for {}x{} image",
        canvas.len(), image.width(), image.height()
    );

    if let Some(image) = image.as_rgba8() {
        for (source, target) in image.as_raw().chunks_exact(4).zip(canvas.chunks_exact_mut(4)) {
            let [r, g, b, _] = source else {
                unreachable!("RGBA chunks are exactly 4 bytes");
            };

            target.copy_from_slice(&[*b, *g, *r, 0]);
        }
        return;
    }

    if let Some(image) = image.as_rgb8() {
        for (source, target) in image.as_raw().chunks_exact(3).zip(canvas.chunks_exact_mut(4)) {
            let [r, g, b] = source else {
                unreachable!("RGB chunks are exactly 3 bytes");
            };

            target.copy_from_slice(&[*b, *g, *r, 0]);
        }
        return;
    }

    for (pos, (_, _, pixel)) in image.pixels().enumerate() {
        let indice = pos * 4;

        let [r, g, b, _] = pixel.0;

        let r = u32::from(r) << 16;
        let g = u32::from(g) << 8;
        let b = u32::from(b);

        canvas[indice..indice + 4].copy_from_slice(&(r | g | b).to_le_bytes());
    }
}
