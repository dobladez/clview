//! Images, decoded in Rust by the `image` crate, and JPEG XL by jxl-oxide: one page each, its
//! size the picture's in pixels, turned the way its EXIF orientation says. Opening reads only
//! the header. A renderer, or a pool, decodes the picture on its first render (on a render
//! thread, for a pool) and keeps it for the renders after, until it goes: the source itself
//! keeps no pixels, so a directory of photos open at once costs little. A picture on disk is
//! read again when it decodes; one from a pipe is kept as it came.

use std::borrow::Cow;
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ::image::imageops::FilterType;
use ::image::metadata::Orientation;
use ::image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};

use super::threads::Threads;
use super::{Again, Backend, Frame, Input, Opened, Pixels, Pool, Render, Reopenable, Request, Source, guarded};
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

    /// Those it decodes, by their signature, or by their names where they have none.
    fn formats(&self) -> Vec<super::Format> {
        let named = |format: ImageFormat| if NAMED.contains(&format) { format.extensions_str() } else { &[] };
        let decoded = DECODED.iter().map(|&(format, name)| super::Format { name, extensions: named(format) });
        decoded.chain([super::Format { name: JPEG_XL, extensions: &[] }]).collect()
    }

    /// The `image` crate, and jxl-oxide for JPEG XL.
    fn credits(&self) -> &[&'static str] {
        &["image", "jxl-oxide"]
    }

    fn options(&self) -> &'static [OptionSpec] {
        &[]
    }

    fn opens(&self, path: &str, head: &[u8]) -> bool {
        format(path, head).is_some()
    }

    fn draws_pictures(&self) -> bool {
        true
    }

    fn open(&self, input: Input, _options: &Parsed<'_>) -> Result<Opened, String> {
        let path = input.path.clone();
        let format = format(&path, &input.head).ok_or_else(|| format!("{path}: not an image"))?;
        let image = match input.reopenable()? {
            Reopenable::Disk(file) => Image::on_disk(Path::new(&path), file, format),
            Reopenable::Bytes(bytes) => Image::in_memory(bytes, format),
            Reopenable::Again(again) => Image::held(again, format),
        };
        Ok(Opened::Pages(Box::new(image.map_err(|e| format!("{path}: {e}"))?)))
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

/// The `image` crate's formats this build decodes, each with its name in `--help`: the ones
/// `Cargo.toml` enables. Kept here rather than asked of `ImageFormat::reading_enabled`, which in
/// image 0.25.10 goes by AVIF's encoder, not its decoder. A picture in another format it knows
/// (AVIF, DDS) is refused with its name.
const DECODED: &[(ImageFormat, &str)] = &[
    (ImageFormat::Png, "PNG"),
    (ImageFormat::Jpeg, "JPEG"),
    (ImageFormat::Gif, "GIF"),
    (ImageFormat::WebP, "WebP"),
    (ImageFormat::Tiff, "TIFF"),
    (ImageFormat::Bmp, "BMP"),
    (ImageFormat::Ico, "ICO"),
    (ImageFormat::Qoi, "QOI"),
    (ImageFormat::Tga, "TGA"),
    (ImageFormat::Pnm, "PNM"),
    (ImageFormat::Hdr, "Radiance HDR"),
    (ImageFormat::OpenExr, "OpenEXR"),
    (ImageFormat::Farbfeld, "farbfeld"),
];

/// The formats a picture has no signature in, known by their names alone, as the `image` crate
/// tells them: TGA.
const NAMED: &[ImageFormat] = &[ImageFormat::Tga];

/// JPEG XL's name in `--help`: decoded by jxl-oxide, not the `image` crate.
const JPEG_XL: &str = "JPEG XL";

/// The format of the picture at `path` that starts with `head`: by its signature, or for one
/// that has none ([`NAMED`]), by its name.
fn format(path: &str, head: &[u8]) -> Option<Format> {
    if is_jxl(head) {
        return Some(Format::Jxl);
    }
    let format = ::image::guess_format(head).ok().or_else(|| ImageFormat::from_path(path).ok().filter(|f| NAMED.contains(f)))?;
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
    encoded: Encoded,
    format: Format,
    /// As shown: turned by its orientation.
    size: (u32, u32),
}

/// Where a picture's bytes are.
enum Encoded {
    /// In a file on disk, read again to decode.
    Disk(PathBuf),
    /// As they came through a pipe, which can't be read again.
    Memory(Vec<u8>),
    /// In another file (an archive), read from it again to decode.
    Held(Again),
}

/// The picture as premultiplied RGBA, decoded by the first render of a renderer or a pool, each
/// of which has its own: the decoder's error, or a panic's, is kept too, so a picture that fails
/// is not decoded again for every render.
type Cache = OnceLock<Result<Arc<RgbaImage>, String>>;

impl Image {
    /// The picture in the file at `path`, whose header is read from `file`, at its start.
    fn on_disk(path: &Path, file: File, format: Format) -> Result<Self, String> {
        let size = header(BufReader::new(file), format)?;
        Ok(Self(Arc::new(Picture { encoded: Encoded::Disk(path.into()), format, size })))
    }

    /// The picture that `again` reads from the file holding it: read once now for its header.
    fn held(again: Again, format: Format) -> Result<Self, String> {
        let size = header(Cursor::new(&within(again()?)?), format)?;
        Ok(Self(Arc::new(Picture { encoded: Encoded::Held(again), format, size })))
    }

    /// The picture in `bytes`.
    fn in_memory(bytes: Vec<u8>, format: Format) -> Result<Self, String> {
        let size = header(Cursor::new(&bytes), format)?;
        Ok(Self(Arc::new(Picture { encoded: Encoded::Memory(bytes), format, size })))
    }
}

/// From the header `reader` starts with: the picture's size as shown, once it is known to be in
/// a format this build decodes and to fit [`MAX_BYTES`].
fn header(reader: impl BufRead + Seek, format: Format) -> Result<(u32, u32), String> {
    if let Format::Image(format) = format
        && !DECODED.iter().any(|&(decoded, _)| decoded == format)
    {
        let name = format.extensions_str().first().map_or_else(|| format!("{format:?}"), |e| e.to_uppercase());
        return Err(format!("{name} pictures aren't decoded by this build of {}", crate::NAME));
    }
    let mut decoder = decoder(reader, format)?;
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
    Ok(if turned { (height, width) } else { (width, height) })
}

impl Source for Image {
    fn pages(&self) -> usize {
        1
    }

    fn size(&self, page: usize) -> Option<(f32, f32)> {
        let (width, height) = self.0.size;
        (page == 0).then_some((width as f32, height as f32))
    }

    fn natural(&self, _page: usize) -> bool {
        true
    }

    fn renderer(&self) -> Result<Box<dyn Render + '_>, String> {
        Ok(Box::new(Renderer { picture: self.0.clone(), decoded: Cache::new() }))
    }

    fn pool(&self, threads: usize) -> Box<dyn Pool> {
        let (picture, decoded) = (self.0.clone(), Arc::new(Cache::new()));
        Box::new(Threads::new(threads, Arc::new(move |request| picture.render(request, &decoded))))
    }
}

struct Renderer {
    picture: Arc<Picture>,
    decoded: Cache,
}

impl Render for Renderer {
    fn render(&mut self, request: Request) -> Frame {
        guarded(request, || self.picture.render(request, &self.decoded))
    }
}

impl Picture {
    fn render(&self, request: Request, decoded: &Cache) -> Frame {
        self.rendered(request, decoded).unwrap_or_else(|e| Frame::failed(request, e))
    }

    fn rendered(&self, request: Request, decoded: &Cache) -> Result<Frame, String> {
        if request.page != 0 {
            return Err(format!("an image has one page, not {}", request.page.saturating_add(1)));
        }
        let decoded = decoded.get_or_init(|| self.bytes().and_then(|bytes| guarded_decode(&bytes, self.format))).clone()?;
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

    /// Its bytes: the ones kept, or the file's again.
    fn bytes(&self) -> Result<Cow<'_, [u8]>, String> {
        match &self.encoded {
            Encoded::Memory(bytes) => Ok(Cow::Borrowed(bytes)),
            Encoded::Disk(path) => read(path).map(Cow::Owned),
            Encoded::Held(again) => again().and_then(within).map(Cow::Owned),
        }
    }
}

/// The file at `path`, as long as it is no larger than [`MAX_BYTES`]: no picture this opens is
/// larger encoded than that.
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path).and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes)).map_err(|e| format!("reading the file again: {e}"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return Err(format!("the file is now more than {} MiB", MAX_BYTES >> 20));
    }
    Ok(bytes)
}

/// `bytes`, read from the file holding a picture, as long as they are no more than
/// [`MAX_BYTES`], as [`read`] reads a file of its own.
fn within(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    match u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        true => Err(format!("the file is more than {} MiB", MAX_BYTES >> 20)),
        false => Ok(bytes),
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
    let mut decoder = decoder(Cursor::new(bytes), format)?;
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

/// A decoder for the picture `reader` reads, held to [`MAX_BYTES`].
fn decoder<'a>(reader: impl BufRead + Seek + 'a, format: Format) -> Result<Box<dyn ImageDecoder + 'a>, String> {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_BYTES);
    let mut decoder: Box<dyn ImageDecoder + 'a> = match format {
        Format::Image(format) => {
            let mut reader = ImageReader::with_format(reader, format);
            reader.limits(limits.clone());
            Box::new(reader.into_decoder().map_err(|e| e.to_string())?)
        }
        Format::Jxl => Box::new(jxl_oxide::integration::JxlDecoder::new(reader).map_err(|e| e.to_string())?),
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
        Image::in_memory(bytes, format)
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

    /// `--help` lists what this build decodes: each format the `image` crate says it is built to
    /// read (asked of all but AVIF, whose answer goes by its encoder), a picture of it as the crate
    /// writes one decoding, and JPEG XL.
    #[test]
    fn the_formats_listed_are_those_this_build_decodes() {
        let asked = |format: &ImageFormat| *format != ImageFormat::Avif;
        let built: Vec<ImageFormat> = ImageFormat::all().filter(|f| asked(f) && f.reading_enabled()).collect();
        let listed: Vec<ImageFormat> = DECODED.iter().map(|&(format, _)| format).collect();
        for format in &built {
            assert!(listed.contains(format), "{format:?} decodes in this build, but isn't listed");
        }
        for format in listed.iter().filter(|f| asked(f)) {
            assert!(built.contains(format), "{format:?} is listed, but this build doesn't decode it");
        }
        let pixels = DynamicImage::ImageRgba8(RgbaImage::from_pixel(3, 2, ::image::Rgba([10, 20, 30, 255])));
        let kinds = [pixels.clone(), pixels.to_rgb8().into(), pixels.to_rgb32f().into(), pixels.to_rgba16().into()];
        for &(format, name) in DECODED {
            // The first kind of pixels the crate writes the format in.
            let picture = kinds.iter().find_map(|kind| {
                let mut out = Cursor::new(Vec::new());
                kind.write_to(&mut out, format).ok().map(|()| out.into_inner())
            });
            let picture = picture.unwrap_or_else(|| panic!("{name}: not written by this build"));
            let image = open(&format!("a.{}", format.extensions_str()[0]), picture).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(render(&image, 1.0).error, None, "{name}");
        }
        let names: Vec<&str> = Images.formats().iter().map(|f| f.name).collect();
        assert_eq!(names.len(), DECODED.len() + 1);
        assert_eq!(names.last(), Some(&"JPEG XL"));
        assert_eq!(open("a.jxl", JXL.to_vec()).map(|image| image.size(0)), Ok(Some((3.0, 2.0))));
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

    /// A picture on disk keeps no bytes and no pixels in its source: each renderer reads the file
    /// when it first renders, and keeps what it decoded for itself.
    #[test]
    fn a_picture_on_disk_is_read_by_each_renderer_and_kept_by_it() {
        let path = std::env::temp_dir().join(format!("clview-image-disk-{}.png", std::process::id()));
        let coloured = |c: u8| png(&[c, c, c, 255].repeat(6), 3, 2);
        std::fs::write(&path, coloured(10)).unwrap();
        let image = Image::on_disk(&path, File::open(&path).unwrap(), Format::Image(ImageFormat::Png)).unwrap();
        let mut first = image.renderer().unwrap();
        let pixel = |frame: Frame| frame.rgba.first().copied();
        assert_eq!(pixel(first.render(Request { page: 0, scale: 1.0, tag: 1 })), Some(10));
        std::fs::write(&path, coloured(200)).unwrap();
        // The first renderer has the picture as it decoded it; a new one reads the file anew.
        assert_eq!(pixel(first.render(Request { page: 0, scale: 2.0, tag: 2 })), Some(10));
        assert_eq!(pixel(render(&image, 1.0)), Some(200));
        std::fs::remove_file(&path).unwrap();
        assert!(render(&image, 1.0).error.is_some_and(|e| e.starts_with("reading the file again: ")));
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
