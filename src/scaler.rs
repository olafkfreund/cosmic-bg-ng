// SPDX-License-Identifier: MPL-2.0

//! Background scaling methods such as fit, stretch, and zoom.

use image::imageops::FilterType;
use image::{DynamicImage, Pixel, RgbaImage};
use std::borrow::Cow;
use std::sync::Arc;

const FIT_BLUR_RADIUS: f32 = 24.0;
const FIT_BLUR_BACKGROUND_MAX_SIZE: u32 = 960;
const FIT_BLUR_FOREGROUND_FEATHER_PX: u32 = 40;
const VIDEO_FIT_BLUR_FOREGROUND_FILTER: fast_image_resize::FilterType =
    fast_image_resize::FilterType::Bilinear;

pub struct FitBlurBgrxWorkspace {
    resizer: fast_image_resize::Resizer,
    foreground: Vec<u8>,
    normalized: Vec<u8>,
}

impl Default for FitBlurBgrxWorkspace {
    fn default() -> Self {
        Self {
            resizer: fast_image_resize::Resizer::new(),
            foreground: Vec::new(),
            normalized: Vec::new(),
        }
    }
}

pub fn fit(
    img: &image::DynamicImage,
    color: &[f32; 3],
    layer_width: u32,
    layer_height: u32,
) -> image::DynamicImage {
    // TODO: convert color to the same format as the input image.
    let mut filled_image =
        image::ImageBuffer::from_pixel(layer_width, layer_height, *image::Rgb::from_slice(color));

    let (w, h) = (img.width(), img.height());

    let ratio = (layer_width as f64 / w as f64).min(layer_height as f64 / h as f64);

    let (new_width, new_height) = (
        (w as f64 * ratio).round() as u32,
        (h as f64 * ratio).round() as u32,
    );

    let resized_image = resize(img, new_width, new_height);

    image::imageops::replace(
        &mut filled_image,
        &resized_image.to_rgb32f(),
        ((layer_width - new_width) / 2).into(),
        ((layer_height - new_height) / 2).into(),
    );

    DynamicImage::from(filled_image)
}

pub fn stretch(
    img: &image::DynamicImage,
    layer_width: u32,
    layer_height: u32,
) -> image::DynamicImage {
    resize(img, layer_width, layer_height)
}

pub fn fit_blur(
    img: &image::DynamicImage,
    layer_width: u32,
    layer_height: u32,
) -> image::DynamicImage {
    let background = image::imageops::blur(
        &zoom(img, layer_width, layer_height).to_rgba8(),
        FIT_BLUR_RADIUS,
    );

    compose_fit_blur(img, &background, layer_width, layer_height)
}

pub fn fit_blur_background_fast(
    img: &image::DynamicImage,
    layer_width: u32,
    layer_height: u32,
) -> RgbaImage {
    let scale = (FIT_BLUR_BACKGROUND_MAX_SIZE as f64 / layer_width.max(layer_height) as f64)
        .min(1.0);
    let background_width = ((layer_width as f64 * scale).round() as u32).max(1);
    let background_height = ((layer_height as f64 * scale).round() as u32).max(1);
    let blur_radius = (FIT_BLUR_RADIUS * scale as f32).max(1.0);

    let small_background = image::imageops::blur(
        &zoom(img, background_width, background_height).to_rgba8(),
        blur_radius,
    );

    if background_width == layer_width && background_height == layer_height {
        small_background
    } else {
        image::imageops::resize(
            &small_background,
            layer_width,
            layer_height,
            FilterType::Triangle,
        )
    }
}

pub fn compose_fit_blur(
    img: &image::DynamicImage,
    background: &RgbaImage,
    layer_width: u32,
    layer_height: u32,
) -> image::DynamicImage {
    let mut background = background.clone();
    let (w, h) = (img.width(), img.height());
    let ratio = (layer_width as f64 / w as f64).min(layer_height as f64 / h as f64);
    let (new_width, new_height) = (
        (w as f64 * ratio).round() as u32,
        (h as f64 * ratio).round() as u32,
    );
    let foreground = resize(img, new_width, new_height).to_rgba8();
    let foreground_x = (layer_width - new_width) / 2;
    let foreground_y = (layer_height - new_height) / 2;

    overlay_fit_blur_foreground(
        &mut background,
        &foreground,
        foreground_x,
        foreground_y,
        layer_width,
        layer_height,
    );

    DynamicImage::ImageRgba8(background)
}

pub fn fit_blur_background_to_bgrx(background: &RgbaImage) -> Arc<[u8]> {
    let mut bgrx = Vec::with_capacity(background.as_raw().len());
    for pixel in background.as_raw().chunks_exact(4) {
        let [r, g, b, _a] = pixel else {
            unreachable!("RGBA chunks are exactly 4 bytes");
        };
        bgrx.extend_from_slice(&[*b, *g, *r, 0]);
    }
    Arc::from(bgrx)
}

#[allow(clippy::too_many_arguments)]
pub fn compose_fit_blur_bgrx_to_canvas(
    canvas: &mut [u8],
    canvas_stride: usize,
    background: &[u8],
    frame: &[u8],
    frame_width: u32,
    frame_height: u32,
    frame_stride: usize,
    layer_width: u32,
    layer_height: u32,
) {
    let mut workspace = FitBlurBgrxWorkspace::default();
    compose_fit_blur_bgrx_to_canvas_with_workspace(
        canvas,
        canvas_stride,
        background,
        frame,
        frame_width,
        frame_height,
        frame_stride,
        layer_width,
        layer_height,
        &mut workspace,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn compose_fit_blur_bgrx_to_canvas_with_workspace(
    canvas: &mut [u8],
    canvas_stride: usize,
    background: &[u8],
    frame: &[u8],
    frame_width: u32,
    frame_height: u32,
    frame_stride: usize,
    layer_width: u32,
    layer_height: u32,
    workspace: &mut FitBlurBgrxWorkspace,
) {
    let layer_width_usize = layer_width as usize;
    let layer_height_usize = layer_height as usize;
    let Some(layer_row_bytes) = layer_width_usize.checked_mul(4) else {
        tracing::warn!("Invalid fit-blur BGRx layer dimensions");
        return;
    };

    if canvas_stride < layer_row_bytes {
        tracing::warn!("Invalid fit-blur BGRx canvas stride");
        return;
    }
    let Some(canvas_len) = required_len(canvas_stride, layer_height_usize, layer_row_bytes) else {
        tracing::warn!("Invalid fit-blur BGRx canvas dimensions");
        return;
    };
    let Some(background_len) = layer_row_bytes.checked_mul(layer_height_usize) else {
        tracing::warn!("Invalid fit-blur BGRx background dimensions");
        return;
    };
    if canvas.len() < canvas_len || background.len() < background_len {
        tracing::warn!("Fit-blur BGRx canvas or background buffer is too small");
        return;
    }

    if frame_width == 0 || frame_height == 0 || layer_width == 0 || layer_height == 0 {
        tracing::warn!("Invalid fit-blur BGRx foreground dimensions");
        // Fill with the blurred background so the surface is never left with
        // uninitialized (recycled) buffer contents.
        copy_background_to_canvas(canvas, canvas_stride, background, layer_row_bytes, layer_height_usize);
        return;
    }

    let (foreground_width, foreground_height) = fit_blur_foreground_size(
        frame_width,
        frame_height,
        layer_width,
        layer_height,
    );

    let foreground_row_bytes = foreground_width as usize * 4;
    let covers_layer = foreground_width == layer_width && foreground_height == layer_height;
    let identity_scale = foreground_width == frame_width && foreground_height == frame_height;

    // Fast path: the fitted video fills the whole layer at native resolution.
    // The blurred background is then fully hidden and the foreground needs no
    // rescale, so a single per-row copy replaces the full-screen background
    // fill, the identity resize, and the per-pixel feathered overlay. This is
    // the common case for full-screen-aspect video (e.g. a 16:9 clip on a 16:9
    // output) and removes ~3 full-frame passes plus the resampler per frame.
    if covers_layer
        && identity_scale
        && copy_bgrx_frame_to_canvas(
            canvas,
            canvas_stride,
            frame,
            frame_width,
            frame_height,
            frame_stride,
        )
    {
        return;
    }

    // The blurred background is only visible where the foreground does not cover
    // the layer, so skip the full-screen copy when the foreground fills it.
    if !covers_layer {
        copy_background_to_canvas(canvas, canvas_stride, background, layer_row_bytes, layer_height_usize);
    }

    // Skip the resampler when the foreground is already at native size; the raw
    // BGRx frame can be overlaid directly.
    let foreground: &[u8] = if identity_scale
        && frame_stride == foreground_row_bytes
        && frame.len() >= foreground_row_bytes * foreground_height as usize
    {
        &frame[..foreground_row_bytes * foreground_height as usize]
    } else {
        resize_bgrx(
            frame,
            frame_width,
            frame_height,
            frame_stride,
            foreground_width,
            foreground_height,
            workspace,
        )
    };
    let foreground_x = (layer_width - foreground_width) / 2;
    let foreground_y = (layer_height - foreground_height) / 2;

    overlay_fit_blur_bgrx_foreground(
        canvas,
        canvas_stride,
        foreground,
        foreground_width,
        foreground_height,
        foreground_x,
        foreground_y,
        layer_width,
        layer_height,
    );
}

fn fit_blur_foreground_size(
    frame_width: u32,
    frame_height: u32,
    layer_width: u32,
    layer_height: u32,
) -> (u32, u32) {
    let ratio =
        (layer_width as f64 / frame_width as f64).min(layer_height as f64 / frame_height as f64);
    (
        (frame_width as f64 * ratio).round() as u32,
        (frame_height as f64 * ratio).round() as u32,
    )
}

/// Copies the full-screen blurred background into the canvas, honouring the
/// canvas stride. The background is stored tightly packed at `layer_row_bytes`.
fn copy_background_to_canvas(
    canvas: &mut [u8],
    canvas_stride: usize,
    background: &[u8],
    layer_row_bytes: usize,
    layer_height: usize,
) {
    for row in 0..layer_height {
        let dst_start = row * canvas_stride;
        let src_start = row * layer_row_bytes;
        canvas[dst_start..dst_start + layer_row_bytes]
            .copy_from_slice(&background[src_start..src_start + layer_row_bytes]);
    }
}

/// Copies a native-resolution BGRx frame straight into the XRGB8888 canvas, one
/// row at a time to honour differing source/destination strides. BGRx and
/// XRGB8888 share byte layout, so no per-pixel conversion is needed. Returns
/// `false` without touching the canvas when the frame buffer is too small, so
/// the caller can fall back to the full compositing path.
fn copy_bgrx_frame_to_canvas(
    canvas: &mut [u8],
    canvas_stride: usize,
    frame: &[u8],
    frame_width: u32,
    frame_height: u32,
    frame_stride: usize,
) -> bool {
    let row_bytes = frame_width as usize * 4;
    let height = frame_height as usize;
    if frame_stride < row_bytes {
        return false;
    }
    let (Some(required_src), Some(required_dst)) = (
        required_len(frame_stride, height, row_bytes),
        required_len(canvas_stride, height, row_bytes),
    ) else {
        return false;
    };
    if frame.len() < required_src || canvas.len() < required_dst {
        return false;
    }

    for row in 0..height {
        let src_start = row * frame_stride;
        let dst_start = row * canvas_stride;
        canvas[dst_start..dst_start + row_bytes]
            .copy_from_slice(&frame[src_start..src_start + row_bytes]);
    }
    true
}

fn resize_bgrx<'a>(
    frame: &[u8],
    frame_width: u32,
    frame_height: u32,
    frame_stride: usize,
    new_width: u32,
    new_height: u32,
    workspace: &'a mut FitBlurBgrxWorkspace,
) -> &'a [u8] {
    let FitBlurBgrxWorkspace {
        resizer,
        foreground,
        normalized,
    } = workspace;
    let Some(source) =
        normalized_bgrx_rows_with_scratch(frame, frame_width, frame_height, frame_stride, normalized)
    else {
        tracing::warn!("Invalid BGRx frame dimensions. Skipping foreground resize.");
        foreground.clear();
        return foreground;
    };

    let Ok(src_image) = fast_image_resize::images::ImageRef::new(
        frame_width,
        frame_height,
        &source,
        fast_image_resize::PixelType::U8x4,
    ) else {
        *foreground = resize_bgrx_fallback(&source, frame_width, frame_height, new_width, new_height);
        return foreground;
    };

    let Some(foreground_len) = (new_width as usize)
        .checked_mul(new_height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        tracing::warn!("Invalid BGRx resize dimensions. Skipping foreground resize.");
        foreground.clear();
        return foreground;
    };
    foreground.resize(foreground_len, 0);

    let Ok(mut dst_image) = fast_image_resize::images::Image::from_slice_u8(
        new_width,
        new_height,
        foreground.as_mut_slice(),
        fast_image_resize::PixelType::U8x4,
    ) else {
        *foreground = resize_bgrx_fallback(&source, frame_width, frame_height, new_width, new_height);
        return foreground;
    };
    let options = fast_image_resize::ResizeOptions {
        algorithm: fast_image_resize::ResizeAlg::Convolution(
            VIDEO_FIT_BLUR_FOREGROUND_FILTER,
        ),
        mul_div_alpha: false,
        ..Default::default()
    };

    if let Err(err) = resizer.resize(&src_image, &mut dst_image, &options) {
        tracing::warn!(?err, "Failed to resize BGRx frame. Falling back.");
        *foreground = resize_bgrx_fallback(&source, frame_width, frame_height, new_width, new_height);
        return foreground;
    }

    drop(dst_image);
    foreground
}

#[cfg(test)]
fn normalized_bgrx_rows(
    frame: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Option<Cow<'_, [u8]>> {
    let row_bytes = (width as usize).checked_mul(4)?;
    let height = height as usize;
    let required = required_len(stride, height, row_bytes)?;
    if stride < row_bytes || frame.len() < required {
        return None;
    }

    if stride == row_bytes {
        return Some(Cow::Borrowed(&frame[..row_bytes * height]));
    }

    let mut compact = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        for pixel in frame[start..start + row_bytes].chunks_exact(4) {
            compact.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    Some(Cow::Owned(compact))
}

fn normalized_bgrx_rows_with_scratch<'a>(
    frame: &'a [u8],
    width: u32,
    height: u32,
    stride: usize,
    scratch: &'a mut Vec<u8>,
) -> Option<Cow<'a, [u8]>> {
    let row_bytes = (width as usize).checked_mul(4)?;
    let height = height as usize;
    let required = required_len(stride, height, row_bytes)?;
    if stride < row_bytes || frame.len() < required {
        return None;
    }

    if stride == row_bytes {
        return Some(Cow::Borrowed(&frame[..row_bytes * height]));
    }

    scratch.clear();
    scratch.reserve(row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        for pixel in frame[start..start + row_bytes].chunks_exact(4) {
            scratch.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    Some(Cow::Borrowed(scratch))
}

fn required_len(stride: usize, height: usize, row_bytes: usize) -> Option<usize> {
    match height {
        0 => Some(0),
        _ => stride.checked_mul(height - 1)?.checked_add(row_bytes),
    }
}

fn resize_bgrx_fallback(
    source: &[u8],
    frame_width: u32,
    frame_height: u32,
    new_width: u32,
    new_height: u32,
) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(source.len());
    for pixel in source.chunks_exact(4) {
        let [b, g, r, _x] = pixel else {
            unreachable!("BGRx chunks are exactly 4 bytes");
        };
        rgba.extend_from_slice(&[*r, *g, *b, 255]);
    }

    let image = image::ImageBuffer::from_raw(frame_width, frame_height, rgba)
        .map(DynamicImage::ImageRgba8)
        .expect("fallback source buffer dimensions are valid");
    let resized = resize(&image, new_width, new_height).to_rgba8();
    fit_blur_background_to_bgrx(&resized).to_vec()
}

#[allow(clippy::too_many_arguments)]
fn overlay_fit_blur_bgrx_foreground(
    canvas: &mut [u8],
    canvas_stride: usize,
    foreground: &[u8],
    foreground_width: u32,
    foreground_height: u32,
    foreground_x: u32,
    foreground_y: u32,
    layer_width: u32,
    layer_height: u32,
) {
    if foreground.is_empty() {
        return;
    }

    let feather = FIT_BLUR_FOREGROUND_FEATHER_PX
        .min(foreground_width / 2)
        .min(foreground_height / 2);

    for y in 0..foreground_height {
        let src_row_start = y as usize * foreground_width as usize * 4;
        let dst_row_start =
            (foreground_y + y) as usize * canvas_stride + foreground_x as usize * 4;

        for x in 0..foreground_width {
            let src_start = src_row_start + x as usize * 4;
            let dst_start = dst_row_start + x as usize * 4;

            let alpha = if feather == 0 {
                255
            } else {
                fit_blur_foreground_alpha(
                    x,
                    y,
                    foreground_width,
                    foreground_height,
                    foreground_x,
                    foreground_y,
                    layer_width,
                    layer_height,
                    feather,
                )
            };

            if alpha == 255 {
                canvas[dst_start..dst_start + 4]
                    .copy_from_slice(&foreground[src_start..src_start + 4]);
                canvas[dst_start + 3] = 0;
                continue;
            }

            for channel in 0..3 {
                canvas[dst_start + channel] = blend_channel(
                    canvas[dst_start + channel],
                    foreground[src_start + channel],
                    alpha,
                );
            }
            canvas[dst_start + 3] = 0;
        }
    }
}

fn overlay_fit_blur_foreground(
    background: &mut RgbaImage,
    foreground: &RgbaImage,
    foreground_x: u32,
    foreground_y: u32,
    layer_width: u32,
    layer_height: u32,
) {
    let foreground_width = foreground.width();
    let foreground_height = foreground.height();
    let feather = FIT_BLUR_FOREGROUND_FEATHER_PX
        .min(foreground_width / 2)
        .min(foreground_height / 2);

    if feather == 0 {
        image::imageops::replace(
            background,
            foreground,
            foreground_x.into(),
            foreground_y.into(),
        );
        return;
    }

    for y in 0..foreground_height {
        for x in 0..foreground_width {
            let alpha = fit_blur_foreground_alpha(
                x,
                y,
                foreground_width,
                foreground_height,
                foreground_x,
                foreground_y,
                layer_width,
                layer_height,
                feather,
            );
            let foreground_pixel = foreground.get_pixel(x, y);
            let background_pixel = background.get_pixel_mut(foreground_x + x, foreground_y + y);

            for channel in 0..3 {
                background_pixel.0[channel] = blend_channel(
                    background_pixel.0[channel],
                    foreground_pixel.0[channel],
                    alpha,
                );
            }
            background_pixel.0[3] = 255;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fit_blur_foreground_alpha(
    x: u32,
    y: u32,
    foreground_width: u32,
    foreground_height: u32,
    foreground_x: u32,
    foreground_y: u32,
    layer_width: u32,
    layer_height: u32,
    feather: u32,
) -> u8 {
    let mut edge_distance = feather;

    if foreground_x > 0 {
        edge_distance = edge_distance.min(x);
    }
    if foreground_x + foreground_width < layer_width {
        edge_distance = edge_distance.min(foreground_width - 1 - x);
    }
    if foreground_y > 0 {
        edge_distance = edge_distance.min(y);
    }
    if foreground_y + foreground_height < layer_height {
        edge_distance = edge_distance.min(foreground_height - 1 - y);
    }

    if edge_distance >= feather {
        return 255;
    }

    let t = edge_distance as f32 / feather as f32;
    let smooth = t * t * (3.0 - 2.0 * t);
    (smooth * 255.0).round() as u8
}

fn blend_channel(background: u8, foreground: u8, alpha: u8) -> u8 {
    let alpha = alpha as u16;
    (((foreground as u16 * alpha) + (background as u16 * (255 - alpha)) + 127) / 255) as u8
}

pub fn zoom(img: &image::DynamicImage, layer_width: u32, layer_height: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());

    let ratio = (layer_width as f64 / w as f64).max(layer_height as f64 / h as f64);

    let (new_width, new_height) = (
        (w as f64 * ratio).round() as u32,
        (h as f64 * ratio).round() as u32,
    );

    let mut new_image = resize(img, new_width, new_height);

    image::imageops::crop(
        &mut new_image,
        (new_width - layer_width) / 2,
        (new_height - layer_height) / 2,
        layer_width,
        layer_height,
    )
    .to_image()
    .into()
}

fn resize(img: &image::DynamicImage, new_width: u32, new_height: u32) -> image::DynamicImage {
    let mut resizer = fast_image_resize::Resizer::new();
    let options = fast_image_resize::ResizeOptions {
        algorithm: fast_image_resize::ResizeAlg::Convolution(
            fast_image_resize::FilterType::Lanczos3,
        ),
        ..Default::default()
    };
    let mut new_image = image::DynamicImage::new(new_width, new_height, img.color());
    if let Err(err) = resizer.resize(img, &mut new_image, &options) {
        tracing::warn!(?err, "Failed to use `fast_image_resize`. Falling back.");
        new_image =
            image::imageops::resize(img, new_width, new_height, FilterType::Lanczos3).into();
    }
    new_image
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn fast_fit_blur_background_matches_layer_size() {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            320,
            180,
            Rgba([255, 0, 0, 255]),
        ));

        let background = fit_blur_background_fast(&image, 3440, 1440);

        assert_eq!(background.width(), 3440);
        assert_eq!(background.height(), 1440);
    }

    #[test]
    fn compose_fit_blur_centers_foreground_over_cached_background() {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            100,
            50,
            Rgba([255, 0, 0, 255]),
        ));
        let background = ImageBuffer::from_pixel(200, 200, Rgba([0, 0, 255, 255]));

        let composed = compose_fit_blur(&image, &background, 200, 200).to_rgba8();

        assert_eq!(composed.get_pixel(100, 100).0, [255, 0, 0, 255]);
        assert_eq!(composed.get_pixel(100, 10).0, [0, 0, 255, 255]);
    }

    #[test]
    fn compose_fit_blur_feathers_only_edges_with_adjacent_background() {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            160,
            90,
            Rgba([255, 0, 0, 255]),
        ));
        let background = ImageBuffer::from_pixel(344, 144, Rgba([0, 0, 255, 255]));

        let composed = compose_fit_blur(&image, &background, 344, 144).to_rgba8();

        assert_eq!(composed.get_pixel(172, 72).0, [255, 0, 0, 255]);
        assert_eq!(composed.get_pixel(44, 72).0, [0, 0, 255, 255]);
        assert_eq!(composed.get_pixel(84, 72).0, [255, 0, 0, 255]);
        assert_eq!(composed.get_pixel(172, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn compose_fit_blur_blends_foreground_feather_with_background() {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            160,
            90,
            Rgba([255, 0, 0, 255]),
        ));
        let background = ImageBuffer::from_pixel(344, 144, Rgba([0, 0, 255, 255]));

        let composed = compose_fit_blur(&image, &background, 344, 144).to_rgba8();

        let feather_pixel = composed.get_pixel(64, 72).0;
        assert!(feather_pixel[0] > 0 && feather_pixel[0] < 255);
        assert_eq!(feather_pixel[1], 0);
        assert!(feather_pixel[2] > 0 && feather_pixel[2] < 255);
        assert_eq!(feather_pixel[3], 255);
    }

    #[test]
    fn compose_fit_blur_bgrx_to_canvas_centers_and_feathers() {
        let background: Vec<u8> = [255, 0, 0, 0].repeat(344 * 144);
        let foreground: Vec<u8> = [0, 0, 255, 255].repeat(160 * 90);
        let mut canvas = vec![0; 344 * 144 * 4];

        compose_fit_blur_bgrx_to_canvas(
            &mut canvas,
            344 * 4,
            &background,
            &foreground,
            160,
            90,
            160 * 4,
            344,
            144,
        );

        let center = (72 * 344 + 172) * 4;
        let outside = (72 * 344 + 44) * 4;
        let feather = (72 * 344 + 64) * 4;

        assert_eq!(&canvas[center..center + 4], &[0, 0, 255, 0]);
        assert_eq!(&canvas[outside..outside + 4], &[255, 0, 0, 0]);
        assert!(canvas[feather] > 0 && canvas[feather] < 255);
        assert_eq!(canvas[feather + 1], 0);
        assert!(canvas[feather + 2] > 0 && canvas[feather + 2] < 255);
        assert_eq!(canvas[feather + 3], 0);
    }

    #[test]
    fn compose_fit_blur_bgrx_tight_zero_x_foreground_keeps_color() {
        let background: Vec<u8> = [255, 0, 0, 0].repeat(8 * 4);
        let foreground: Vec<u8> = [0, 0, 255, 0].repeat(2);
        let mut canvas = vec![0; 8 * 4 * 4];

        compose_fit_blur_bgrx_to_canvas(
            &mut canvas,
            8 * 4,
            &background,
            &foreground,
            2,
            1,
            2 * 4,
            8,
            4,
        );

        let center = (2 * 8 + 4) * 4;
        assert_eq!(&canvas[center..center + 4], &[0, 0, 255, 0]);
    }

    #[test]
    fn compose_fit_blur_bgrx_workspace_reuses_across_frame_layouts() {
        let mut workspace = FitBlurBgrxWorkspace::default();

        let background_a: Vec<u8> = [255, 0, 0, 0].repeat(8 * 4);
        let foreground_a: Vec<u8> = [0, 0, 255, 0].repeat(2);
        let mut canvas_a = vec![0; 8 * 4 * 4];

        compose_fit_blur_bgrx_to_canvas_with_workspace(
            &mut canvas_a,
            8 * 4,
            &background_a,
            &foreground_a,
            2,
            1,
            2 * 4,
            8,
            4,
            &mut workspace,
        );

        let center_a = (2 * 8 + 4) * 4;
        assert_eq!(&canvas_a[center_a..center_a + 4], &[0, 0, 255, 0]);

        let background_b: Vec<u8> = [0, 255, 0, 0].repeat(6 * 8);
        let foreground_b = [
            0, 0, 255, 0, 0, 0, 255, 0, 99, 99, 99, 99, 0, 0, 255, 0, 0, 0, 255, 0, 99,
            99, 99, 99,
        ];
        let mut canvas_b = vec![0; 6 * 8 * 4];

        compose_fit_blur_bgrx_to_canvas_with_workspace(
            &mut canvas_b,
            6 * 4,
            &background_b,
            &foreground_b,
            2,
            2,
            12,
            6,
            8,
            &mut workspace,
        );

        let center_b = (4 * 6 + 3) * 4;
        assert_eq!(canvas_b[center_b], 0);
        assert!(canvas_b[center_b + 1] < 255);
        assert!(canvas_b[center_b + 2] > 0);
        assert_eq!(canvas_b[center_b + 3], 0);
        assert_eq!(&canvas_b[..4], &[0, 255, 0, 0]);
    }

    #[test]
    fn compose_fit_blur_bgrx_full_cover_copies_frame_without_background_or_feather() {
        let mut workspace = FitBlurBgrxWorkspace::default();

        // Layer and frame are identical 4x2 BGRx buffers, so the fitted video
        // fills the whole layer at native resolution (the fast path).
        let layer_width = 4u32;
        let layer_height = 2u32;
        let row_bytes = layer_width as usize * 4;
        let pixels = layer_width as usize * layer_height as usize;

        // A distinctive background that must NOT appear anywhere if the fast
        // path skips both the background fill and the feathered overlay.
        let background: Vec<u8> = [255, 0, 0, 0].repeat(pixels);
        let frame: Vec<u8> = [7, 8, 9, 0].repeat(pixels);
        let mut canvas = vec![0u8; row_bytes * layer_height as usize];

        compose_fit_blur_bgrx_to_canvas_with_workspace(
            &mut canvas,
            row_bytes,
            &background,
            &frame,
            layer_width,
            layer_height,
            row_bytes,
            layer_width,
            layer_height,
            &mut workspace,
        );

        // Every pixel — including the corners the feather would otherwise blend
        // with the red background — equals the source frame untouched.
        assert_eq!(canvas, frame);
    }

    #[test]
    fn compose_fit_blur_bgrx_full_cover_honours_canvas_stride() {
        let mut workspace = FitBlurBgrxWorkspace::default();

        // Same full-cover fast path, but with canvas padding (stride larger than
        // the visible row) to ensure the per-row copy respects the stride.
        let layer_width = 4u32;
        let layer_height = 2u32;
        let row_bytes = layer_width as usize * 4;
        let canvas_stride = row_bytes + 8;
        let pixels = layer_width as usize * layer_height as usize;

        let background: Vec<u8> = [255, 0, 0, 0].repeat(pixels);
        let frame: Vec<u8> = [7, 8, 9, 0].repeat(pixels);
        let mut canvas = vec![0u8; canvas_stride * layer_height as usize];

        compose_fit_blur_bgrx_to_canvas_with_workspace(
            &mut canvas,
            canvas_stride,
            &background,
            &frame,
            layer_width,
            layer_height,
            row_bytes,
            layer_width,
            layer_height,
            &mut workspace,
        );

        for row in 0..layer_height as usize {
            let dst = row * canvas_stride;
            let src = row * row_bytes;
            assert_eq!(&canvas[dst..dst + row_bytes], &frame[src..src + row_bytes]);
        }
    }

    #[test]
    fn compose_fit_blur_bgrx_to_canvas_rejects_short_buffers() {
        let background = vec![0; 4];
        let foreground = vec![0; 4];
        let mut canvas = vec![0; 4];

        compose_fit_blur_bgrx_to_canvas(
            &mut canvas,
            4,
            &background,
            &foreground,
            2,
            1,
            8,
            8,
            4,
        );

        assert_eq!(canvas, vec![0; 4]);
    }

    #[test]
    fn normalized_bgrx_rows_borrows_tightly_packed_frame() {
        let frame: Vec<u8> = [1, 2, 3, 4].repeat(4);

        let normalized = normalized_bgrx_rows(&frame, 2, 2, 2 * 4).unwrap();

        assert!(matches!(normalized, Cow::Borrowed(_)));
        assert_eq!(&*normalized, &frame);
    }

    #[test]
    fn normalized_bgrx_rows_compacts_strided_frame() {
        let frame = [
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16, 0, 0,
        ];

        let normalized = normalized_bgrx_rows(&frame, 2, 2, 10).unwrap();

        assert!(matches!(normalized, Cow::Owned(_)));
        assert_eq!(
            &*normalized,
            &[1, 2, 3, 255, 5, 6, 7, 255, 9, 10, 11, 255, 13, 14, 15, 255]
        );
    }

    #[test]
    fn normalized_bgrx_rows_rejects_short_frame() {
        let frame = [1, 2, 3, 4];

        assert!(normalized_bgrx_rows(&frame, 2, 1, 8).is_none());
    }
}
