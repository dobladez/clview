//! Images, decoded in Rust by the `image` crate, and JPEG XL by jxl-oxide: one page each, its
//! size the picture's in pixels, turned the way its EXIF orientation says. Opening reads only
//! the header; the picture is decoded by the first render, on a render thread, and kept for the
//! renders after it.

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use ::image::imageops::FilterType;
use ::image::metadata::Orientation;
use ::image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};

use super::threads::Threads;
use super::{Backend, Frame, Input, Pixels, Pool, Render, Request, Source, guarded};
use crate::args::{OptionSpec, Parsed};

/// Images, decoded by the `image` crate.
pub(super) struct Images;

impl Backend for Images {
    fn name(&self) -> &'static str {
        "image"
    }

    fn shows(&self) -> &'static str {
        "images"
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        format(path, head).is_some()
    }

    fn open(&self, input: Input<'_>, _options: &Parsed<'_>) -> Result<Box<dyn Source>, String> {
        let path = input.path;
        let bytes = input.bytes()?;
        let format = format(path, &bytes).ok_or_else(|| format!("{path}: not an image"))?;
        Ok(Box::new(Image::open(bytes, format).map_err(|e| format!("{path}: {e}"))?))
    }
}

/// The most bytes a decoded picture may take, and so the largest it opens: 512 MiB, about 134
/// megapixels in RGBA.
const MAX_BYTES: u64 = 512 << 20;

/// How a picture is decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Image(ImageFormat),
    Jxl,
}

/// The `image` crate's formats this build decodes: the ones `Cargo.toml` enables. Kept here
/// rather than asked of `ImageFormat::reading_enabled`, which in image 0.25.10 goes by AVIF's
/// encoder, not its decoder. A picture in another format it knows (AVIF, DDS) is refused with
/// its name.
const DECODED: &[ImageFormat] = &[
    ImageFormat::Png,
    ImageFormat::Jpeg,
    ImageFormat::Gif,
    ImageFormat::WebP,
    ImageFormat::Tiff,
    ImageFormat::Bmp,
    ImageFormat::Ico,
    ImageFormat::Qoi,
    ImageFormat::Tga,
    ImageFormat::Pnm,
    ImageFormat::Hdr,
    ImageFormat::OpenExr,
    ImageFormat::Farbfeld,
];

/// The format of the picture at `path` that starts with `head`: by its signature, or for TGA,
/// which has none, by its name.
fn format(path: &str, head: &[u8]) -> Option<Format> {
    if is_jxl(head) {
        return Some(Format::Jxl);
    }
    let format = ::image::guess_format(head).ok().or_else(|| ImageFormat::from_path(path).ok().filter(|f| *f == ImageFormat::Tga))?;
    Some(Format::Image(format))
}

/// Whether `head` starts a JPEG XL file: a bare codestream, or the ISO BMFF container
/// (ISO/IEC 18181-1 and -2).
fn is_jxl(head: &[u8]) -> bool {
    head.starts_with(&[0xFF, 0x0A]) || head.starts_with(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A])
}

/// An open picture.
struct Image(Arc<Picture>);

struct Picture {
    bytes: Vec<u8>,
    format: Format,
    /// As shown: turned by its orientation.
    size: (u32, u32),
    /// Premultiplied RGBA, decoded by the first render: the decoder's error, or a panic's, is
    /// kept too, so a picture that fails is not decoded again for every render.
    decoded: OnceLock<Result<Arc<RgbaImage>, String>>,
}

impl Image {
    /// Reads the header: the size, and whether the picture fits [`MAX_BYTES`].
    fn open(bytes: Vec<u8>, format: Format) -> Result<Self, String> {
        if let Format::Image(format) = format
            && !DECODED.contains(&format)
        {
            let name = format.extensions_str().first().map_or_else(|| format!("{format:?}"), |e| e.to_uppercase());
            return Err(format!("{name} pictures aren't decoded by this build of {}", crate::NAME));
        }
        let mut decoder = decoder(&bytes, format)?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 {
            return Err(format!("a {width} × {height} picture has no pixels to show"));
        }
        if u64::from(width).saturating_mul(u64::from(height)).saturating_mul(4) > MAX_BYTES {
            return Err(format!("{width} × {height} pixels is more than {} shows (at most {} MiB decoded)", crate::NAME, MAX_BYTES >> 20));
        }
        let turned = matches!(
            orientation(&mut *decoder, format),
            Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
        );
        let size = if turned { (height, width) } else { (width, height) };
        drop(decoder);
        Ok(Self(Arc::new(Picture { bytes, format, size, decoded: OnceLock::new() })))
    }
}

impl Source for Image {
    fn pages(&self) -> usize {
        1
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        let (width, height) = self.0.size;
        (page == 0).then_some((width as f32, height as f32))
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer(self.0.clone())))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool + '_> {
        let picture = self.0.clone();
        Box::new(Threads::new(threads, Arc::new(move |request| picture.render(request))))
    }
}

struct Renderer(Arc<Picture>);

impl Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.0.render(request))
    }
}

impl Picture {
    fn render(&self, request: Request) -> Frame {
        self.rendered(request).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request) -> Result<Frame, String> {
        if request.page != 0 {
            return Err(format!("an image has one page, not {}", request.page.saturating_add(1)));
        }
        let decoded = self.decoded.get_or_init(|| guarded_decode(&self.bytes, self.format)).clone()?;
        let (width, height) = decoded.dimensions();
        // The header promised pixels; a decoder that then makes none leaves nothing to scale.
        if width == 0 || height == 0 {
            return Err("the picture decoded to no pixels".into());
        }
        let (to_width, to_height) = (scaled(width, request.scale)?, scaled(height, request.scale)?);
        if u64::from(to_width).saturating_mul(u64::from(to_height)).saturating_mul(4) > MAX_BYTES {
            return Err(format!("{to_width} × {to_height} pixels is too large to render"));
        }
        let rgba = if (to_width, to_height) == (width, height) {
            Pixels::new(Decoded(decoded))
        } else {
            // Premultiplied, so a transparent pixel's colour doesn't bleed into its neighbours.
            let filter = if to_width < width { FilterType::Triangle } else { FilterType::CatmullRom };
            Pixels::new(::image::imageops::resize(&*decoded, to_width, to_height, filter).into_raw())
        };
        Ok(Frame { request, width: to_width, height: to_height, rgba, error: None, warnings: Vec::new() })
    }
}

/// The picture decoded once and shared by every render at full size, without a copy.
struct Decoded(Arc<RgbaImage>);

impl AsRef<[u8]> for Decoded {
    fn as_ref(&self) -> &[u8] {
        self.0.as_raw()
    }
}

/// `pixels` at `scale`, rounded, at least one.
fn scaled(pixels: u32, scale: f32) -> Result<u32, String> {
    let scaled = (f64::from(pixels) * f64::from(scale)).round();
    if !(scale > 0.0 && scaled.is_finite() && scaled <= f64::from(u32::MAX)) {
        return Err(format!("can't render at scale {scale}"));
    }
    // In range: checked just above.
    Ok((scaled as u32).max(1))
}

/// [`decode`], a decoder's panic made an error.
fn guarded_decode(bytes: &[u8], format: Format) -> Result<Arc<RgbaImage>, String> {
    std::panic::catch_unwind(|| decode(bytes, format)).unwrap_or_else(|_| Err("the decoder failed on this picture".into())).map(Arc::new)
}

/// The picture as premultiplied RGBA, turned the way its orientation says.
fn decode(bytes: &[u8], format: Format) -> Result<RgbaImage, String> {
    let mut decoder = decoder(bytes, format)?;
    let orientation = orientation(&mut *decoder, format);
    let mut picture = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    picture.apply_orientation(orientation);
    let mut rgba = picture.into_rgba8();
    premultiply(&mut rgba);
    Ok(rgba)
}

/// How the decoded pixels are still to be turned: as EXIF says, or not at all for a JPEG XL,
/// which jxl-oxide hands out turned by the orientation in its codestream (which rules over its
/// EXIF's, ISO/IEC 18181-2). An orientation that can't be read leaves the picture as it is.
fn orientation(decoder: &mut dyn ImageDecoder, format: Format) -> Orientation {
    match format {
        Format::Jxl => Orientation::NoTransforms,
        Format::Image(_) => decoder.orientation().unwrap_or(Orientation::NoTransforms),
    }
}

/// A decoder for `bytes`, held to [`MAX_BYTES`].
fn decoder(bytes: &[u8], format: Format) -> Result<Box<dyn ImageDecoder + '_>, String> {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_BYTES);
    let mut decoder: Box<dyn ImageDecoder + '_> = match format {
        Format::Image(format) => {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            reader.limits(limits.clone());
            Box::new(reader.into_decoder().map_err(|e| e.to_string())?)
        }
        Format::Jxl => Box::new(jxl_oxide::integration::JxlDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?),
    };
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    Ok(decoder)
}

/// Straight alpha to premultiplied, as a [`Frame`] holds it.
fn premultiply(rgba: &mut [u8]) {
    for [r, g, b, a] in rgba.as_chunks_mut::<4>().0 {
        if *a < u8::MAX {
            for c in [r, g, b] {
                // At most 255 × 255 / 255.
                *c = ((u16::from(*c) * u16::from(*a) + 127) / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::image::codecs::{
        bmp::BmpEncoder, gif::GifEncoder, ico::IcoEncoder, jpeg::JpegEncoder, png::PngEncoder, pnm::PnmEncoder, qoi::QoiEncoder, tga::TgaEncoder,
        tiff::TiffEncoder, webp::WebPEncoder,
    };
    use ::image::{ExtendedColorType, ImageEncoder, ImageResult};

    use super::*;

    /// The picture `tests/data` holds: red, green and blue over white, black and brown.
    const PIXELS: [[u8; 3]; 6] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255], [0, 0, 0], [128, 64, 32]];
    const JXL: &[u8] = include_bytes!("../../tests/data/three-by-two.jxl");
    const AVIF: &[u8] = include_bytes!("../../tests/data/three-by-two.avif");
    const TURNED_JXL: &[u8] = include_bytes!("../../tests/data/turned.jxl");

    fn opaque() -> Vec<u8> {
        PIXELS.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect()
    }

    fn written(write: impl FnOnce(&mut Cursor<Vec<u8>>) -> ImageResult<()>) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        write(&mut out).unwrap();
        out.into_inner()
    }

    fn png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        written(|out| PngEncoder::new(out).write_image(rgba, width, height, ExtendedColorType::Rgba8))
    }

    fn open(name: &str, bytes: Vec<u8>) -> Result<Image, String> {
        let format = format(name, &bytes).ok_or("not an image")?;
        Image::open(bytes, format)
    }

    fn render(image: &Image, scale: f32) -> Frame {
        image.renderer().unwrap().render(Request { page: 0, scale, tag: 7 })
    }

    #[test]
    fn every_format_decodes_to_the_same_pixels() {
        let rgba = opaque();
        let rgba8 = ExtendedColorType::Rgba8;
        let cases = [
            ("a.png", png(&rgba, 3, 2)),
            ("a.bmp", written(|out| BmpEncoder::new(out).write_image(&rgba, 3, 2, rgba8))),
            ("a.tiff", written(|out| TiffEncoder::new(out).write_image(&rgba, 3, 2, rgba8))),
            ("a.gif", written(|out| GifEncoder::new(out).encode(&rgba, 3, 2, rgba8))),
            ("a.webp", written(|out| WebPEncoder::new_lossless(out).write_image(&rgba, 3, 2, rgba8))),
            ("a.qoi", written(|out| QoiEncoder::new(out).write_image(&rgba, 3, 2, rgba8))),
            ("a.ico", written(|out| IcoEncoder::new(out).write_image(&rgba, 3, 2, rgba8))),
            ("a.ppm", written(|out| PnmEncoder::new(out).write_image(&PIXELS.concat(), 3, 2, ExtendedColorType::Rgb8))),
            ("a.tga", written(|out| TgaEncoder::new(out).write_image(&rgba, 3, 2, rgba8))),
            ("a.jxl", JXL.to_vec()),
        ];
        for (name, bytes) in cases {
            let image = open(name, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(image.size(0), Some((3.0, 2.0)), "{name}");
            let frame = render(&image, 1.0);
            assert_eq!((frame.error.as_deref(), frame.width, frame.height), (None, 3, 2), "{name}");
            assert_eq!(&*frame.rgba, &rgba[..], "{name}");
        }
    }

    #[test]
    fn a_picture_is_known_by_its_signature_not_its_name() {
        let png = png(&opaque(), 3, 2);
        assert_eq!(format("photo.jpg", &png), Some(Format::Image(ImageFormat::Png)));
        assert_eq!(format("a.png", b"%PDF-1.7\n"), None);
        assert_eq!(format("a.png", b"hello"), None);
        assert_eq!(format("a.png", b""), None);
        assert_eq!(format("x", JXL), Some(Format::Jxl), "a bare JPEG XL codestream");
        assert_eq!(format("x", b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl "), Some(Format::Jxl), "JPEG XL in its container");
        // TGA has no signature: only its name says what it is.
        let tga = written(|out| TgaEncoder::new(out).write_image(&opaque(), 3, 2, ExtendedColorType::Rgba8));
        assert_eq!(format("a.TGA", &tga), Some(Format::Image(ImageFormat::Tga)));
        assert_eq!(format("a.bin", &tga), None);
    }

    /// An image this build doesn't decode is still an image: refused with its format's name, not
    /// as a file nothing shows.
    #[test]
    fn a_picture_in_a_format_not_decoded_is_refused_by_name() {
        assert_eq!(format("a.avif", AVIF), Some(Format::Image(ImageFormat::Avif)));
        assert_eq!(open("a.avif", AVIF.to_vec()).err(), Some(format!("AVIF pictures aren't decoded by this build of {}", crate::NAME)));
    }

    #[test]
    fn transparent_pixels_are_premultiplied() {
        let image = open("a.png", png(&[200, 100, 50, 128, 10, 20, 30, 0, 9, 8, 7, 255], 3, 1)).unwrap();
        assert_eq!(&*render(&image, 1.0).rgba, &[100, 50, 25, 128, 0, 0, 0, 0, 9, 8, 7, 255]);
    }

    /// A 16 × 8 JPEG, red on the left and blue on the right, with EXIF `orientation`.
    fn photo(orientation: u16) -> Vec<u8> {
        let rgb: Vec<u8> = (0..8).flat_map(|_| (0..16).flat_map(|x| if x < 8 { [255, 0, 0] } else { [0, 0, 255] })).collect();
        let jpeg = written(|out| JpegEncoder::new_with_quality(out, 95).write_image(&rgb, 16, 8, ExtendedColorType::Rgb8));
        // The orientation tag (0x0112), a SHORT, alone in a big-endian TIFF IFD.
        let tiff = [&b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01"[..], &orientation.to_be_bytes(), b"\0\0\0\0\0\0"].concat();
        let app1 = [&b"Exif\0\0"[..], &tiff].concat();
        let length = u16::try_from(app1.len() + 2).unwrap().to_be_bytes();
        [&jpeg[..2], b"\xff\xe1", &length, &app1, &jpeg[2..]].concat()
    }

    /// Whether the pixel at `x`, `y` of `frame` is mostly red, or mostly blue.
    fn red(frame: &Frame, x: u32, y: u32) -> bool {
        let at = ((y * frame.width + x) * 4) as usize;
        let [r, _, b] = [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]];
        assert!(r.abs_diff(b) > 150, "({x}, {y}) is neither: {:?}", &frame.rgba[at..at + 4]);
        r > b
    }

    #[test]
    fn a_photo_is_turned_the_way_its_exif_orientation_says() {
        let upright = open("a.jpg", photo(1)).unwrap();
        assert_eq!(upright.size(0), Some((16.0, 8.0)));
        let frame = render(&upright, 1.0);
        assert!(red(&frame, 2, 4) && !red(&frame, 13, 4));
        // 6: turned a quarter clockwise to be seen, so the left comes to the top.
        let turned = open("a.jpg", photo(6)).unwrap();
        assert_eq!(turned.size(0), Some((8.0, 16.0)));
        let frame = render(&turned, 1.0);
        assert_eq!((frame.width, frame.height), (8, 16));
        assert!(red(&frame, 4, 2) && !red(&frame, 4, 13));
        // 8: a quarter anticlockwise, so the left comes to the bottom.
        let frame = render(&open("a.jpg", photo(8)).unwrap(), 1.0);
        assert!(!red(&frame, 4, 2) && red(&frame, 4, 13));
    }

    /// A JPEG XL has its orientation in the codestream, which jxl-oxide applies itself; its EXIF
    /// says the same, and is not applied a second time.
    #[test]
    fn a_jpeg_xl_is_turned_once() {
        let turned = open("a.jxl", TURNED_JXL.to_vec()).unwrap();
        assert_eq!(turned.size(0), Some((8.0, 16.0)));
        let frame = render(&turned, 1.0);
        assert_eq!((frame.error.as_deref(), frame.width, frame.height), (None, 8, 16));
        assert!(red(&frame, 4, 2) && !red(&frame, 4, 13));
    }

    #[test]
    fn a_render_is_the_decoded_picture_at_its_scale() {
        let image = open("a.png", png(&opaque(), 3, 2)).unwrap();
        let size = |scale| {
            let frame = render(&image, scale);
            (frame.error, frame.width, frame.height, frame.rgba.len())
        };
        assert_eq!(size(2.0), (None, 6, 4, 6 * 4 * 4));
        assert_eq!(size(0.5), (None, 2, 1, 2 * 4));
        assert_eq!(size(0.001), (None, 1, 1, 4), "never less than a pixel");
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(size(scale).0.is_some_and(|e| e.contains("can't render at scale")), "{scale}");
        }
        let second = image.renderer().unwrap().render(Request { page: 1, scale: 1.0, tag: 7 });
        assert!(second.error.is_some_and(|e| e.contains("one page")));
        assert_eq!(image.size(1), None);
    }

    #[test]
    fn a_picture_too_large_or_broken_is_refused_when_opened() {
        // A BMP header that promises 20000 × 20000 pixels, 1.6 GB decoded, and no pixels.
        let huge = [
            &b"BM"[..],
            &[0; 8],
            &54u32.to_le_bytes(),
            &40u32.to_le_bytes(),
            &20_000i32.to_le_bytes(),
            &20_000i32.to_le_bytes(),
            &1u16.to_le_bytes(),
            &24u16.to_le_bytes(),
            &[0; 24],
        ]
        .concat();
        assert_eq!(open("a.bmp", huge).err(), Some(format!("20000 × 20000 pixels is more than {} shows (at most 512 MiB decoded)", crate::NAME)));
        let mut broken = png(&opaque(), 3, 2);
        broken.truncate(8);
        broken.extend_from_slice(b"not a PNG chunk at all");
        assert!(open("a.png", broken).is_err());
    }

    #[test]
    fn a_picture_that_fails_to_decode_fails_its_page_every_time() {
        // The header is whole and the pixels are cut short.
        let mut cut = png(&(0..64 * 64 * 4).map(|i| (i * 7 % 251) as u8).collect::<Vec<_>>(), 64, 64);
        cut.truncate(cut.len() / 2);
        let image = open("a.png", cut).unwrap();
        let first = render(&image, 1.0).error;
        assert!(first.is_some());
        assert_eq!(render(&image, 2.0).error, first);
    }
}
