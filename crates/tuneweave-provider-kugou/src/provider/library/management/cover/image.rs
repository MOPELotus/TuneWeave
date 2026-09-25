//! Bounded local conversion to the native client's square JPEG cover format.
use super::*;
use ::image::{
    ImageDecoder, ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use std::io::Cursor;

pub(super) const MAX_INPUT: usize = 20 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
static PREPARATION_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

#[cfg(test)]
pub(crate) async fn pause_preparation() -> tokio::sync::SemaphorePermit<'static> {
    PREPARATION_SLOTS.acquire_many(2).await.unwrap()
}

pub(super) struct Prepared {
    pub(super) jpeg: Vec<u8>,
    pub(super) output_size: u32,
    pub(super) crop: (u32, u32, u32),
}

pub(super) fn validate(r: &ImageUploadRequest) -> Result<()> {
    if r.data.is_empty()
        || r.data.len() > MAX_INPUT
        || r.filename.is_empty()
        || r.filename.len() > 255
        || r.filename.chars().any(char::is_control)
        || r.content_type.len() > 128
        || r.content_type.chars().any(char::is_control)
        || r.image_size == Some(0)
        || (r.image_size.is_none() && (r.crop_x.is_some() || r.crop_y.is_some()))
    {
        return Err(invalid_image());
    }
    let format = ::image::guess_format(&r.data).map_err(|_| invalid_image())?;
    let expected = match r
        .content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/gif" => ImageFormat::Gif,
        "image/bmp" => ImageFormat::Bmp,
        _ => return Err(invalid_image()),
    };
    if format != expected {
        return Err(invalid_image());
    }
    Ok(())
}

pub(super) async fn prepare(r: &ImageUploadRequest, client: KugouLoginClient) -> Result<Prepared> {
    let max_output = match client {
        KugouLoginClient::Standard => 1000,
        KugouLoginClient::Concept => 400,
        KugouLoginClient::Web => {
            return Err(unsupported("KuGou Web cover preparation is unavailable"));
        }
    };
    let permit = PREPARATION_SLOTS
        .acquire()
        .await
        .map_err(|_| invalid_image())?;
    let r = r.clone();
    // No network or file operations occur in this bounded blocking job. Dropping
    // the caller cannot cause a late preparation result to upload anything.
    tokio::task::spawn_blocking(move || {
        // A blocking decoder keeps its slot even when its async caller is cancelled.
        let _permit = permit;
        decode(&r, max_output)
    })
    .await
    .map_err(|_| invalid_image())?
}

fn decode(r: &ImageUploadRequest, max_output: u32) -> Result<Prepared> {
    let format = ::image::guess_format(&r.data).map_err(|_| invalid_image())?;
    let mut reader = ImageReader::with_format(Cursor::new(&r.data), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| invalid_image())?;
    let (w, h) = decoder.dimensions();
    if w < 400 || h < 400 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(invalid_image());
    }
    let orientation = decoder.orientation().map_err(|_| invalid_image())?;
    let mut source = ::image::DynamicImage::from_decoder(decoder).map_err(|_| invalid_image())?;
    source.apply_orientation(orientation);
    let (w, h) = (source.width(), source.height());
    let crop = match r.image_size {
        Some(size) => (r.crop_x.unwrap_or(0), r.crop_y.unwrap_or(0), size),
        None => {
            let size = w.min(h);
            ((w - size) / 2, (h - size) / 2, size)
        }
    };
    if crop.2 < 400
        || crop.0.checked_add(crop.2).is_none_or(|v| v > w)
        || crop.1.checked_add(crop.2).is_none_or(|v| v > h)
    {
        return Err(invalid_image());
    }
    let cropped = source.crop_imm(crop.0, crop.1, crop.2, crop.2);
    let mut rgba = cropped.to_rgba8();
    // JPEG has no alpha. Composite transparent pixels over white explicitly.
    for p in rgba.pixels_mut() {
        let a = u32::from(p[3]);
        for c in &mut p.0[..3] {
            *c = ((u32::from(*c) * a + 255 * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
    let output_size = crop.2.min(max_output);
    let rgb = ::image::DynamicImage::ImageRgba8(rgba)
        .resize_exact(output_size, output_size, FilterType::Triangle)
        .to_rgb8();
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, 100)
        .encode_image(&rgb)
        .map_err(|_| invalid_image())?;
    if jpeg.len() > MAX_INPUT {
        return Err(invalid_image());
    }
    Ok(Prepared {
        jpeg,
        output_size,
        crop,
    })
}

fn invalid_image() -> TuneWeaveError {
    invalid(
        "KuGou cover requires a bounded JPEG, PNG, GIF or BMP image with a square crop of at least 400 pixels within its dimensions",
    )
}
