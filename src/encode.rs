//! Rendered pages → PNG, for the terminal and for a document to hold as a picture. A renderer hands
//! out premultiplied RGBA; the kitty graphics protocol takes straight alpha, as a PNG (`f=100`) or as
//! raw pixels (`f=32`).

/// How hard PNG encoding works at making the file small. The pixels come out the same either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PngEffort {
    /// Deflate level 3 with adaptive filtering: the smaller file. On rendered pages it writes
    /// files the size of the `png` crate's default (level 6) in a third to a fifth of the time.
    #[default]
    Small,
    /// Two to five times faster: the least on a text page, where it writes about twice the
    /// bytes. For images shown at once, where encoding time is felt and the bytes go no further
    /// than the screen.
    Fast,
}

/// Premultiplied RGBA → straight RGBA, in place. A trailing partial pixel is left as it is.
pub(crate) fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

/// Premultiplied RGBA → PNG (straight alpha; RGB when no pixel is translucent, as a rendered
/// page drawn on white never is), compressed with `effort`.
pub(crate) fn encode_png(width: u32, height: u32, premultiplied: &[u8], effort: PngEffort) -> Result<Vec<u8>, String> {
    check_raster(width, height, premultiplied)?;
    let pixels = premultiplied.as_chunks::<4>().0;
    let opaque = pixels.iter().all(|px| px[3] == 255);
    let (color, data) = if opaque {
        (png::ColorType::Rgb, pixels.iter().flat_map(|px| [px[0], px[1], px[2]]).collect::<Vec<u8>>())
    } else {
        let mut rgba = premultiplied.to_vec();
        unpremultiply(&mut rgba);
        (png::ColorType::Rgba, rgba)
    };
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    match effort {
        PngEffort::Small => {
            enc.set_deflate_compression(png::DeflateCompression::Level(3));
            enc.set_filter(png::Filter::Adaptive);
        }
        PngEffort::Fast => enc.set_compression(png::Compression::Fast),
    }
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&data).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// Whether `premultiplied` holds exactly `width` × `height` RGBA pixels, at least one.
pub(crate) fn check_raster(width: u32, height: u32, premultiplied: &[u8]) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err(format!("a {width}×{height} raster has no pixels"));
    }
    let expected = usize::try_from(width).ok().zip(usize::try_from(height).ok()).and_then(|(w, h)| w.checked_mul(h)?.checked_mul(4));
    match expected {
        Some(n) if n == premultiplied.len() => Ok(()),
        Some(n) => Err(format!("a {width}×{height} raster is {n} bytes of RGBA, got {}", premultiplied.len())),
        None => Err(format!("a {width}×{height} raster is too large")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        pixels.truncate(info.buffer_size());
        (info.width, info.height, pixels)
    }

    #[test]
    fn fast_png_holds_the_same_pixels_as_small() {
        // A page-like raster: white, with rows of dark "text", one half-transparent pixel.
        let (w, h) = (64u32, 48u32);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for y in (4..h).step_by(6) {
            for x in 4..w - 4 {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 3].copy_from_slice(&[20, 20, 30]);
            }
        }
        rgba[..4].copy_from_slice(&[64, 0, 0, 128]);
        let small = encode_png(w, h, &rgba, PngEffort::Small).unwrap();
        let fast = encode_png(w, h, &rgba, PngEffort::Fast).unwrap();
        assert_eq!(decode(&fast), decode(&small));
        // Straight alpha out: the premultiplied (64, 0, 0, 128) is (128, 0, 0, 128).
        assert_eq!(decode(&fast).2[..4], [128, 0, 0, 128]);
    }

    /// Only the compression differs: a fast PNG of an opaque raster is RGB too, and a buffer
    /// that doesn't fit its size is refused as it is for a small one.
    #[test]
    fn fast_png_writes_opaque_rasters_as_rgb_and_checks_the_raster() {
        let rgba = [[255, 255, 255, 255], [0, 0, 0, 255], [200, 30, 90, 255]];
        let fast = encode_png(3, 1, rgba.as_flattened(), PngEffort::Fast).unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(&fast)).read_info().unwrap();
        assert_eq!(reader.info().color_type, png::ColorType::Rgb);
        assert_eq!(decode(&fast), decode(&encode_png(3, 1, rgba.as_flattened(), PngEffort::Small).unwrap()));
        for effort in [PngEffort::Small, PngEffort::Fast] {
            assert!(encode_png(1, 1, &[10, 20, 30, 255, 0], effort).is_err(), "a partial pixel after");
            assert!(encode_png(0, 1, &[], effort).is_err(), "no pixels");
            assert!(encode_png(u32::MAX, u32::MAX, &[], effort).is_err(), "too large to size");
        }
    }

    #[test]
    fn unpremultiply_leaves_opaque_clear_and_partial_pixels() {
        let mut px = vec![64, 0, 200, 128, 10, 20, 30, 255, 0, 0, 0, 0, 255, 128, 0, 1, 9, 9];
        unpremultiply(&mut px);
        // Half-covered red doubles; opaque and clear stay; a channel past the alpha clamps; the
        // two bytes after the last whole pixel are left alone.
        assert_eq!(px, [128, 0, 255, 128, 10, 20, 30, 255, 0, 0, 0, 0, 255, 255, 0, 1, 9, 9]);
    }
}
