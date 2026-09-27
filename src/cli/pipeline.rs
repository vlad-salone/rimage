#[cfg(any(feature = "avif", feature = "webp", feature = "svg", feature = "tiff"))]
use std::io::{Seek, SeekFrom};
use std::{collections::BTreeMap, fs::File, path::Path};

#[cfg(feature = "avif")]
use std::io::Read;

#[cfg(feature = "resize")]
use crate::cli::preprocessors::ResizeValue;
use crate::cli::utils::jpeg::JfifDensity;
use clap::ArgMatches;
#[cfg(feature = "avif")]
use rimage::codecs::avif::AvifEncoder;
#[cfg(feature = "mozjpeg")]
use rimage::codecs::mozjpeg::MozJpegEncoder;
#[cfg(feature = "oxipng")]
use rimage::codecs::oxipng::OxiPngEncoder;
#[cfg(feature = "svg")]
use rimage::codecs::svg::SvgDecoder;
#[cfg(all(feature = "svg", not(feature = "resize")))]
use rimage::codecs::svg::SvgOptions;
#[cfg(feature = "webp")]
use rimage::codecs::webp::WebPEncoder;
use zune_core::{bytestream::ZByteWriterTrait, options::DecoderOptions, options::EncoderOptions};
use zune_image::{
    codecs::{
        ImageFormat, farbfeld::FarbFeldEncoder, jpeg::JpegEncoder, jpeg_xl::JxlEncoder,
        png::PngEncoder, ppm::PPMEncoder, qoi::QoiEncoder,
    },
    errors::ImageErrors,
    image::Image,
    metadata::AlphaState,
    traits::{EncoderTrait, OperationsTrait},
};
use zune_imageprocs::premul_alpha::PremultiplyAlpha;

/// Decoder options shared by every path in [`decode`].
///
/// The defaults differ from what this program wants in two ways.
///
/// First, zune decodes *every* frame of an animated PNG or JXL. We only ever
/// re-encode a single still image, and an animation holds one full-size buffer
/// per frame, so decoding the frames we are about to discard is pure memory
/// waste. Asking for the first frame only keeps peak memory proportional to one
/// image.
///
/// Second, the default `max_width`/`max_height` of 16384 is the ceiling the
/// size pre-check reports, and it is deliberately *not* raised here. The
/// underlying codecs advertise more (libjpeg allows 65500), but the decoder
/// refuses a larger image from its header before the codec runs, so pinning a
/// bigger number in [`rimage::limits::format_caps`] would only make the
/// pre-check pass files that fail to decode. The two numbers are the same
/// constant on purpose: raising one without the other reopens that gap.
fn decode_options() -> DecoderOptions {
    DecoderOptions::default()
        .png_set_decode_animated(false)
        .jxl_set_decode_animated(false)
}

/// Reject an image whose header declares dimensions the machine cannot hold.
///
/// This runs *before* any decoding so an oversized file fails with a message
/// naming the ceiling instead of an allocation abort somewhere inside a codec.
/// The header is all that is read, so the check costs a few kilobytes even for
/// an image that is gigabytes when decoded.
///
/// `output` is the format the image is being converted *to*, not the one on
/// disk. It matters because the encoder's scratch buffers are the largest term
/// in the budget and their number is a property of the encoder: screening a
/// JPEG-to-AVIF conversion with JPEG's own cost admits images several times
/// larger than the AVIF encoder can hold. The pixel layout is likewise taken
/// from the widest the input format allows rather than assumed, because the
/// screen cannot see the real one yet.
///
/// Returns `Ok(())` when the format has no readable dimensions for us (an SVG
/// render target, for instance, which is bounded separately) or when they fit.
/// The failure is a [`RimageError`] rather than an [`ImageErrors`] because the
/// violation is resolved here and would otherwise have to be re-parsed out of a
/// string to be reported.
#[cfg(feature = "limits")]
#[allow(unused_variables)]
fn check_input_limits(
    path: &Path,
    matches: &ArgMatches,
    output: rimage::limits::ImageFormatId,
    concurrency: usize,
) -> Result<(), rimage::error::RimageError> {
    use rimage::limits::{ImageFormatId, LimitSet, PipelineCost, SystemBudget};

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let format = ImageFormatId::from_extension(extension);

    // Only worth probing for formats we can size up front. Anything else is
    // rejected later by the decoder or by the SVG point budget. A file that
    // cannot even be opened is left for the decode step to report, so the
    // missing-file message keeps coming from one place.
    let Ok(reader) = File::open(path) else {
        return Ok(());
    };

    let Some((width, height)) = probe_dimensions(reader, format) else {
        return Ok(());
    };

    let (depth, colorspace) = format.max_pixel_layout();
    let budget = SystemBudget::probe(concurrency);
    let limits = LimitSet::for_input(
        format,
        depth,
        colorspace,
        &budget,
        PipelineCost::for_conversion(format, output),
    );

    limits.check(width, height).map_err(|violation| {
        rimage::error::input_size_limit(path, format, Some((width, height)), violation)
    })
}

/// Read just the dimensions from an image header.
///
/// Every format rimage can decode is probed here where a header read can settle
/// the size. Rejecting an oversized input up front is what turns it into a
/// message naming the ceiling instead of an allocation abort inside the codec.
///
/// The two probes that cannot simply read a fixed struct are the two container
/// formats: AVIF keeps its dimensions in an ISO-BMFF property box, and TIFF in
/// an IFD whose location depends on the header. Both are walked rather than
/// scanned for a byte pattern, because a pattern would also match inside
/// compressed image data and report the wrong size.
#[cfg(feature = "limits")]
fn probe_dimensions<R: std::io::Read>(
    mut reader: R,
    format: rimage::limits::ImageFormatId,
) -> Option<(u64, u64)> {
    use rimage::limits::ImageFormatId;

    match format {
        ImageFormatId::Jpeg => {
            let prefix = read_prefix(&mut reader, JPEG_HEADER_PROBE_BYTES)?;
            jpeg_dimensions(&prefix)
        }
        ImageFormatId::WebP => {
            // libwebp exposes a bitstream-features probe that parses the RIFF
            // container and the frame header without decoding any pixels.
            #[cfg(feature = "webp")]
            {
                let prefix = read_prefix(&mut reader, WEBP_HEADER_PROBE_BYTES)?;
                let features = webp::BitstreamFeatures::new(&prefix)?;
                Some((features.width() as u64, features.height() as u64))
            }

            #[cfg(not(feature = "webp"))]
            {
                let _ = &mut reader;
                None
            }
        }
        ImageFormatId::Png => {
            // The IHDR chunk holds both dimensions as big-endian `u32` and is
            // required to be the first chunk (PNG spec § 11.2.2), so a fixed
            // 16-byte prefix settles the size.
            let prefix = read_prefix(&mut reader, PNG_HEADER_PROBE_BYTES)?;
            png_dimensions(&prefix)
        }
        ImageFormatId::Avif => {
            let prefix = read_prefix(&mut reader, CONTAINER_HEADER_PROBE_BYTES)?;
            avif_dimensions(&prefix)
        }
        ImageFormatId::Tiff => {
            let prefix = read_prefix(&mut reader, CONTAINER_HEADER_PROBE_BYTES)?;
            tiff_dimensions(&prefix)
        }
        _ => None,
    }
}

/// Read the width and height from a JPEG Start-Of-Frame segment.
///
/// The dimensions are parsed straight from the marker rather than through
/// `zune_jpeg::JpegDecoder::decode_headers`, because that call applies the
/// decoder's own `max_width`/`max_height` and fails on exactly the images this
/// check exists to report. Reading the marker keeps the probe independent of
/// whatever ceiling the decoder happens to enforce.
///
/// The walk follows JPEG marker segments (ITU-T T.81 § B.2): each is a
/// two-byte marker then a two-byte big-endian length covering the length field
/// itself. Segments that carry no length (`RSTn`, `TEM`, and the standalone
/// `SOI`/`EOI` markers) would desynchronise the walk, so encountering one is
/// treated as "cannot read this file" rather than guessed at.
#[cfg(feature = "limits")]
fn jpeg_dimensions(data: &[u8]) -> Option<(u64, u64)> {
    /// Markers that introduce entropy-coded data; the frame header always
    /// precedes the first of them, so reaching one means it was not found.
    const START_OF_SCAN: u8 = 0xDA;
    /// Markers carrying no length field.
    const STANDALONE_MARKERS: [u8; 6] = [0x01, 0xD0, 0xD1, 0xD2, 0xD8, 0xD9];

    // A JPEG file begins with SOI.
    if data.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }

    let mut at = 2;
    while at + 4 <= data.len() {
        // Markers are introduced by 0xFF; fill bytes of additional 0xFF are
        // permitted before the marker code.
        if data[at] != 0xFF {
            return None;
        }

        let mut marker = data[at + 1];
        while marker == 0xFF {
            at += 1;
            marker = *data.get(at + 1)?;
        }

        if STANDALONE_MARKERS.contains(&marker) {
            return None;
        }
        if marker == START_OF_SCAN {
            return None;
        }

        let length = u16::from_be_bytes(data.get(at + 2..at + 4)?.try_into().ok()?) as usize;
        // The length covers itself, so it is never smaller than two.
        if length < 2 {
            return None;
        }

        // SOF0..SOF15 carry the frame header, except the four that are not
        // frame headers at all: DHT (0xC4), JPG (0xC8), and DAC (0xCC).
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            // Payload: precision(1), height(2), width(2).
            let payload = at + 4;
            let height = u16::from_be_bytes(data.get(payload + 1..payload + 3)?.try_into().ok()?);
            let width = u16::from_be_bytes(data.get(payload + 3..payload + 5)?.try_into().ok()?);

            if width == 0 || height == 0 {
                return None;
            }

            return Some((u64::from(width), u64::from(height)));
        }

        at += 2 + length;
    }

    None
}

/// Bytes of a file read to recover a PNG `IHDR` chunk.
///
/// The signature is 8 bytes, then a 4-byte length, a 4-byte type, and the
/// 13-byte `IHDR` payload whose first eight bytes are the dimensions.
#[cfg(feature = "limits")]
const PNG_HEADER_PROBE_BYTES: usize = 32;

/// Read the width and height from a PNG `IHDR` chunk.
///
/// PNG dimensions are 32-bit and the chunk is required to come first, but the
/// decoder applies its own ceiling of 16384 from `DecoderOptions`, so an image
/// can be well within the format's capability and still be rejected. Probing
/// here is what turns that refusal into a message naming the ceiling.
#[cfg(feature = "limits")]
fn png_dimensions(data: &[u8]) -> Option<(u64, u64)> {
    /// The eight-byte signature every PNG starts with (PNG spec § 5.2).
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    // Signature, then the chunk length and type, then the payload begins.
    const IHDR_PAYLOAD: usize = 16;

    if data.get(0..8)? != SIGNATURE {
        return None;
    }

    // A conforming file's first chunk is `IHDR`; if it is not, this is not a
    // file whose dimensions can be trusted from a prefix.
    if data.get(12..16)? != b"IHDR" {
        return None;
    }

    let width = u32::from_be_bytes(data.get(IHDR_PAYLOAD..IHDR_PAYLOAD + 4)?.try_into().ok()?);
    let height = u32::from_be_bytes(
        data.get(IHDR_PAYLOAD + 4..IHDR_PAYLOAD + 8)?
            .try_into()
            .ok()?,
    );

    if width == 0 || height == 0 {
        return None;
    }

    Some((u64::from(width), u64::from(height)))
}

/// Bytes of a file read to recover a container header.
///
/// AVIF puts its `ispe` property box before the image data, and a TIFF IFD sits
/// at an offset given in the first eight bytes, so both are reachable from the
/// front of the file. The bound keeps refusing an oversized input cheap: the
/// prefix is read, never the payload.
#[cfg(feature = "limits")]
const CONTAINER_HEADER_PROBE_BYTES: usize = 64 * 1024;

/// Read the width and height from an AVIF `ispe` property box.
///
/// AVIF is ISO-BMFF (ISO 14496-12), so the dimensions live in the
/// `ImageSpatialExtent` property of the `meta` box rather than in a fixed
/// header. The boxes are walked by their declared sizes so a byte sequence that
/// merely looks like `ispe` inside compressed data is never mistaken for the
/// property.
///
/// Returns `None` for a malformed or truncated file: the decoder reports that
/// better than a pre-check could.
#[cfg(feature = "limits")]
fn avif_dimensions(data: &[u8]) -> Option<(u64, u64)> {
    /// A box header is a 4-byte big-endian size followed by a 4-byte type.
    const BOX_HEADER: usize = 8;

    fn read_u32(data: &[u8], at: usize) -> Option<u32> {
        let bytes = data.get(at..at + 4)?;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Walk the boxes in `data[range]` looking for one with type `wanted`,
    /// returning the range of its *payload*.
    fn find_box(
        data: &[u8],
        mut start: usize,
        end: usize,
        wanted: &[u8; 4],
    ) -> Option<(usize, usize)> {
        while start + BOX_HEADER <= end {
            let size = read_u32(data, start)?;

            // A size of 1 means a 64-bit size follows the type; a size of 0
            // means the box runs to the end of the enclosing range. Neither is
            // produced for the boxes this function looks for, so rather than
            // half-support them, refuse and let the decoder handle the file.
            if size == 0 || size == 1 {
                return None;
            }

            let size = size as usize;
            let payload = start + BOX_HEADER;
            let box_end = start.checked_add(size)?;
            if box_end > end {
                return None;
            }

            if data.get(start + 4..start + 8)? == wanted {
                return Some((payload, box_end));
            }

            start = box_end;
        }

        None
    }

    // `ftyp` is not consulted: the caller already chose this probe from the
    // file extension, and re-checking the brand here would only duplicate it.
    // `meta` is a plain box, so hunt it at the top level.
    let (meta_start, meta_end) = find_box(data, 0, data.len(), b"meta")?;

    // `meta` is a FullBox: 4 bytes of version and flags precede its children.
    let (iprp_start, iprp_end) = find_box(data, meta_start + 4, meta_end, b"iprp")?;
    let (ipco_start, ipco_end) = find_box(data, iprp_start, iprp_end, b"ipco")?;

    // `ispe` is a FullBox whose payload is version/flags then width and height,
    // each a 32-bit big-endian integer (ISO 14496-12 § 12.1.4).
    let (ispe_start, _) = find_box(data, ipco_start, ipco_end, b"ispe")?;
    let width = read_u32(data, ispe_start + 4)?;
    let height = read_u32(data, ispe_start + 8)?;

    // A zero extent is not a size to reject on; it means the probe misread the
    // container, and a wrong rejection is worse than no pre-check.
    if width == 0 || height == 0 {
        return None;
    }

    Some((u64::from(width), u64::from(height)))
}

/// Read the width and height from a TIFF image file directory.
///
/// Both byte orders are handled, as are both value types the specification
/// allows for these tags: a `SHORT` when the dimension fits in 16 bits and a
/// `LONG` otherwise. Anything else is left to the decoder.
#[cfg(feature = "limits")]
fn tiff_dimensions(data: &[u8]) -> Option<(u64, u64)> {
    /// Tag numbers from TIFF 6.0 § 8: `ImageWidth` and `ImageLength`.
    const TAG_IMAGE_WIDTH: u16 = 0x0100;
    const TAG_IMAGE_LENGTH: u16 = 0x0101;

    /// TIFF type codes; only these two are valid for the tags above.
    const TYPE_SHORT: u16 = 3;
    const TYPE_LONG: u16 = 4;

    const IFD_ENTRY_SIZE: usize = 12;

    // The first two bytes give the byte order, the next two must be 42, and the
    // following four hold the offset of the first IFD (TIFF 6.0 § 2).
    let little_endian = match data.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };

    let u16_at = |at: usize| -> Option<u16> {
        let bytes: [u8; 2] = data.get(at..at + 2)?.try_into().ok()?;
        Some(if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
        Some(if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };

    if u16_at(2)? != 42 {
        return None;
    }

    let ifd = u32_at(4)? as usize;
    let entry_count = u16_at(ifd)? as usize;

    let mut width = None;
    let mut height = None;

    for index in 0..entry_count {
        let entry = ifd + 2 + index * IFD_ENTRY_SIZE;
        let tag = u16_at(entry)?;
        if tag != TAG_IMAGE_WIDTH && tag != TAG_IMAGE_LENGTH {
            continue;
        }

        let field_type = u16_at(entry + 2)?;
        // The count is required to be 1 for these tags; a value other than that
        // means this is not the dimension field it appears to be. It is a
        // 32-bit field, so reading it as 16 bits would see only the high half
        // on a big-endian file and reject every one of them.
        if u32_at(entry + 4)? != 1 {
            return None;
        }

        // TIFF 6.0 § 2 stores a value narrower than four bytes left-justified
        // in the value field, so a `SHORT` occupies the first two bytes under
        // either byte order. Those two bytes are then decoded in the file's
        // order; treating the field as a truncated `LONG` would give the right
        // answer little-endian and zero big-endian.
        let value = match (field_type, little_endian) {
            (TYPE_SHORT, true) => u32::from(u16::from_le_bytes(
                data.get(entry + 8..entry + 10)?.try_into().ok()?,
            )),
            (TYPE_SHORT, false) => u32::from(u16::from_be_bytes(
                data.get(entry + 8..entry + 10)?.try_into().ok()?,
            )),
            (TYPE_LONG, _) => u32_at(entry + 8)?,
            _ => return None,
        };

        if tag == TAG_IMAGE_WIDTH {
            width = Some(value);
        } else {
            height = Some(value);
        }
    }

    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            Some((u64::from(width), u64::from(height)))
        }
        _ => None,
    }
}

/// Read up to `limit` bytes from the front of `reader`.
///
/// Returns `None` when nothing could be read. A short read is not an error: a
/// file too small to hold a header simply is not pre-checked, and the decoder
/// gets to report the real problem.
#[cfg(feature = "limits")]
fn read_prefix<R: std::io::Read>(reader: &mut R, limit: usize) -> Option<Vec<u8>> {
    let mut prefix = vec![0u8; limit];
    let mut filled = 0;
    while filled < prefix.len() {
        match reader.read(&mut prefix[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return None,
        }
    }

    if filled == 0 {
        return None;
    }

    prefix.truncate(filled);
    Some(prefix)
}

/// Bytes of a file read to recover a JPEG frame header.
///
/// The frame header follows the application segments (EXIF, ICC, XMP), which
/// can be large but are almost never larger than this. A file whose header
/// falls beyond the prefix simply is not pre-checked, and the decoder reports
/// the problem instead.
#[cfg(feature = "limits")]
const JPEG_HEADER_PROBE_BYTES: usize = 64 * 1024;

/// Bytes of a file read to recover a WebP bitstream header.
///
/// The RIFF header and the frame header precede the compressed payload, so a
/// small prefix is enough for every WebP variant.
#[cfg(all(feature = "limits", feature = "webp"))]
const WEBP_HEADER_PROBE_BYTES: usize = 64 * 1024;

/// The size screen below takes the concurrency as a parameter rather than
/// deriving it: `main` is the only place that knows both the `--threads`
/// request and how many inputs there are, and the per-image ceiling is the
/// memory divided by the smaller of the two. Re-deriving it here would be a
/// second, silently different source of truth for the number that decides
/// whether an image is "too large".
#[allow(unused_mut, unused_variables)]
pub fn decode<P: AsRef<Path>>(
    f: P,
    matches: &ArgMatches,
    output: rimage::limits::ImageFormatId,
    concurrency: usize,
) -> Result<Image, rimage::error::RimageError> {
    // The size pre-check already knows which ceiling it broke, so it produces
    // the structured error directly instead of round-tripping through a string.
    #[cfg(feature = "limits")]
    check_input_limits(f.as_ref(), matches, output, concurrency)?;

    Image::open_with_options(f.as_ref(), decode_options())
        .or_else(|e| decode_with_fallback(f.as_ref(), matches, output, concurrency, e))
        .map_err(|e| classify_decode_failure(f.as_ref(), matches, &e))
}

/// Pixel budget an SVG rasterisation may cover, derived from the machine.
///
/// The SVG decoder has its own conservative constant for standalone use, but
/// this program can probe the machine, so the render target is bounded by the
/// same memory model every other format uses rather than by a fixed 512 MiB.
/// Returns `None` when the `limits` feature is off, which makes the decoder
/// fall back to its own constant.
#[cfg(feature = "svg")]
#[allow(unused_variables)]
fn svg_pixel_budget(
    matches: &ArgMatches,
    output: rimage::limits::ImageFormatId,
    concurrency: usize,
) -> Option<u64> {
    #[cfg(feature = "limits")]
    {
        use rimage::limits::{ImageFormatId, LimitSet, PipelineCost, SystemBudget};

        let (depth, colorspace) = ImageFormatId::Svg.max_pixel_layout();
        let budget = SystemBudget::probe(concurrency);
        let limits = LimitSet::for_input(
            ImageFormatId::Svg,
            depth,
            colorspace,
            &budget,
            // The render target has to survive the encoder as well, so the
            // same conversion cost applies here as to any other input.
            PipelineCost::for_conversion(ImageFormatId::Svg, output),
        );

        Some(limits.max_pixels)
    }

    #[cfg(not(feature = "limits"))]
    {
        let _ = (matches, output, concurrency);
        None
    }
}

/// Retry the decode with the decoders `zune_image` does not own.
///
/// Split out of [`decode`] so the fallback chain stays readable and so the
/// conversion to [`rimage::error::RimageError`] happens in exactly one place.
#[allow(unused_variables)]
fn decode_with_fallback(
    path: &Path,
    matches: &ArgMatches,
    output: rimage::limits::ImageFormatId,
    concurrency: usize,
    e: ImageErrors,
) -> Result<Image, ImageErrors> {
    {
        if matches!(e, ImageErrors::ImageDecoderNotImplemented(_)) {
            #[cfg(any(feature = "avif", feature = "webp", feature = "svg", feature = "tiff"))]
            let mut file = File::open(path)?;

            #[cfg(feature = "svg")]
            {
                if path
                    .extension()
                    .is_some_and(|f| f.eq_ignore_ascii_case("svg") | f.eq_ignore_ascii_case("svgz"))
                {
                    let resources_dir = path.parent().map(Path::to_path_buf);
                    let pixel_budget = svg_pixel_budget(matches, output, concurrency);

                    #[cfg(feature = "resize")]
                    let decoder = SvgDecoder::try_new_with_resize_and_budget(
                        file,
                        resources_dir,
                        pixel_budget,
                        |size| svg_target_size(matches, size),
                    )?;

                    #[cfg(not(feature = "resize"))]
                    let decoder = SvgDecoder::try_new_with_options(
                        file,
                        SvgOptions {
                            resources_dir,
                            target_size: None,
                            pixel_budget,
                        },
                    )?;

                    return Image::from_decoder(decoder);
                }

                file.seek(SeekFrom::Start(0))?;
            }

            #[cfg(feature = "avif")]
            {
                let mut file_content = vec![];

                file.read_to_end(&mut file_content)?;
                file.seek(SeekFrom::Start(0))?;

                if rimage::codecs::avif::is_avif(&file_content) {
                    use rimage::codecs::avif::AvifDecoder;

                    let decoder = AvifDecoder::try_new(file)?;

                    return Image::from_decoder(decoder);
                };
                file.seek(SeekFrom::Start(0))?;
            }

            #[cfg(feature = "webp")]
            {
                if path
                    .extension()
                    .is_some_and(|f| f.eq_ignore_ascii_case("webp"))
                {
                    use rimage::codecs::webp::WebPDecoder;

                    let decoder = WebPDecoder::try_new_with_options(file, decode_options())?;

                    return Image::from_decoder(decoder);
                }

                file.seek(SeekFrom::Start(0))?;
            }

            #[cfg(feature = "tiff")]
            {
                if path
                    .extension()
                    .is_some_and(|f| f.eq_ignore_ascii_case("tiff") | f.eq_ignore_ascii_case("tif"))
                {
                    use rimage::codecs::tiff::TiffDecoder;

                    let decoder = TiffDecoder::try_new(file)?;

                    return Image::from_decoder(decoder);
                }

                file.seek(SeekFrom::Start(0))?;
            }

            return Err(ImageErrors::ImageDecoderNotImplemented(
                ImageFormat::Unknown,
            ));
        }

        Err(e)
    }
}

/// Turn a decode failure into the structured, side-tagged form the CLI reports.
///
/// The resize context is deliberately not reconstructed here. `classify_input`
/// uses it only to name the requested dimensions in an
/// `ImageOperationNotImplemented("resize")` failure, and the only resize
/// failures reachable at decode time are the SVG render-target ones, whose
/// message already names the offending size. Passing `None` keeps this
/// function honest instead of inventing a reason it did not observe; the
/// classification still reports it as an input failure on the right format.
fn classify_decode_failure(
    path: &Path,
    _matches: &ArgMatches,
    error: &ImageErrors,
) -> rimage::error::RimageError {
    rimage::error::classify_input(path, error, None)
}

#[cfg(all(feature = "svg", feature = "resize"))]
fn svg_target_size(
    matches: &ArgMatches,
    size: (usize, usize),
) -> Result<Option<(u32, u32)>, ImageErrors> {
    use crate::cli::preprocessors::ResizeValue;

    let Some(values) = matches.get_many::<ResizeValue>("resize") else {
        return Ok(None);
    };

    let downscale = matches.get_flag("downscale") && !matches.get_flag("no-downscale");
    let upscale = matches.get_flag("upscale") && !matches.get_flag("no-upscale");

    let first_other = first_other_index(matches);
    let plan = resize_plan(
        values
            .into_iter()
            .zip(matches.indices_of("resize").into_iter().flatten())
            .map(|(value, idx)| (idx, value))
            .take_while(|(idx, _)| *idx < first_other),
        size,
        downscale,
        upscale,
    );

    if plan.is_empty() {
        return Ok(None);
    }

    let final_size = plan.last().map(|(_, size)| *size).unwrap_or(size);
    let width = u32::try_from(final_size.0).map_err(|_| {
        ImageErrors::ImageDecodeErrors(format!(
            "SVG target width {} exceeds the maximum supported dimension of {}",
            final_size.0,
            u32::MAX
        ))
    })?;
    let height = u32::try_from(final_size.1).map_err(|_| {
        ImageErrors::ImageDecodeErrors(format!(
            "SVG target height {} exceeds the maximum supported dimension of {}",
            final_size.1,
            u32::MAX
        ))
    })?;

    Ok(Some((width, height)))
}

/// Returns the index of the first non-resize preprocessing element.
///
/// SVG vector resizing can only be folded into the decode render target for
/// the resize steps that come before every quantization operation and before
/// every true `--premultiply` flag. Steps at or after this index must run as
/// ordinary raster resize operations so command-line order is preserved.
#[cfg(feature = "resize")]
fn first_other_index(matches: &ArgMatches) -> usize {
    let first_premultiply = matches.get_many::<bool>("premultiply").and_then(|values| {
        values
            .into_iter()
            .zip(matches.indices_of("premultiply")?)
            .find_map(|(value, idx)| if *value { Some(idx) } else { None })
    });

    #[cfg(feature = "quantization")]
    let first_quantization = matches
        .indices_of("quantization")
        .and_then(|mut indices| indices.next());
    #[cfg(not(feature = "quantization"))]
    let first_quantization: Option<usize> = None;

    [first_premultiply, first_quantization]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(usize::MAX)
}

/// Plans a chain of resize operations, returning only the steps that are not
/// skipped by the direction flags or by already matching the current size.
///
/// The same logic is used for raster images (which get a physical [`Resize`]
/// operation for every planned step) and for SVG inputs (which use the final
/// planned size as the vector render target).
#[cfg(feature = "resize")]
fn resize_plan<'a>(
    values: impl Iterator<Item = (usize, &'a ResizeValue)>,
    mut size: (usize, usize),
    downscale: bool,
    upscale: bool,
) -> Vec<(usize, (usize, usize))> {
    let mut plan = Vec::new();

    values.for_each(|(idx, value)| {
        let (w, h) = value.map_dimensions(size.0, size.1);
        log::trace!("setup resize {value} on index {idx}");

        // Skip if the image is already the desired size
        // or if both downscale and upscale are disabled
        if (!downscale && !upscale) || (w == size.0 && h == size.1) {
            log::trace!("skip resize to {w}x{h} on index {idx}");
            return;
        }

        if !downscale && (w <= size.0 || h <= size.1) {
            log::trace!("downscaling disabled, skip resize to {w}x{h} on index {idx}");
            return;
        }

        if !upscale && (w >= size.0 || h >= size.1) {
            log::trace!("upscaling disabled, skip resize to {w}x{h} on index {idx}");
            return;
        }

        plan.push((idx, (w, h)));
        size = (w, h);
    });

    plan
}

#[allow(unused_variables)]
#[allow(unused_mut)]
pub fn operations(
    matches: &ArgMatches,
    img: &Image,
    skip_resize: bool,
) -> BTreeMap<usize, Box<dyn OperationsTrait>> {
    let mut map: BTreeMap<usize, Box<dyn OperationsTrait>> = BTreeMap::new();

    #[cfg(feature = "resize")]
    {
        use crate::cli::preprocessors::ResizeFilter;
        use fast_image_resize::ResizeAlg;
        use rimage::operations::resize::Resize;

        if let Some(values) = matches.get_many::<ResizeValue>("resize") {
            let filter = matches.get_one::<ResizeFilter>("filter");

            let downscale = matches.get_flag("downscale") && !matches.get_flag("no-downscale");
            let upscale = matches.get_flag("upscale") && !matches.get_flag("no-upscale");

            log::debug!("downscale: {downscale}, upscale: {upscale}");

            let first_other = first_other_index(matches);
            let plan = resize_plan(
                values
                    .into_iter()
                    .zip(matches.indices_of("resize").into_iter().flatten())
                    .map(|(value, idx)| (idx, value))
                    .filter(|(idx, _)| !skip_resize || *idx >= first_other),
                img.dimensions(),
                downscale,
                upscale,
            );

            for (idx, (w, h)) in plan {
                map.insert(
                    idx,
                    Box::new(Resize::new(
                        w,
                        h,
                        filter
                            .copied()
                            .map(Into::<ResizeAlg>::into)
                            .unwrap_or_default(),
                    )),
                );
            }
        }
    }

    #[cfg(feature = "quantization")]
    {
        use rimage::operations::quantize::Quantize;

        if let Some(values) = matches.get_many::<u8>("quantization") {
            let dithering = matches.get_one::<u8>("dithering");

            values
                .into_iter()
                .zip(matches.indices_of("quantization").into_iter().flatten())
                .for_each(|(value, idx)| {
                    log::trace!("setup quantization {value} on index {idx}");

                    map.insert(
                        idx,
                        Box::new(Quantize::new(*value, dithering.map(|q| *q as f32 / 100.))),
                    );
                })
        }
    }

    if let Some(values) = matches.get_many::<bool>("premultiply") {
        values
            .into_iter()
            .zip(matches.indices_of("premultiply").into_iter().flatten())
            .for_each(|(value, idx)| {
                // Position-sensitive flags inject a trailing default `false`
                // occurrence when the flag is absent from the command line,
                // which must stay silent.
                if !*value {
                    return;
                }

                if let Some(op) = map.get(&(idx + 2)) {
                    log::trace!("setup alpha premultiply for {}", op.name());

                    map.insert(
                        idx,
                        Box::new(PremultiplyAlpha::new(AlphaState::PreMultiplied)),
                    );

                    // If a subsequent operation already occupies idx+3,
                    // log a warning and skip the un-premultiply insertion
                    // rather than aborting the process (which would happen
                    // with `assert!` under `panic = "abort"` in release).
                    match map.entry(idx + 3) {
                        std::collections::btree_map::Entry::Occupied(_) => {
                            log::warn!(
                                "premultiply at index {idx}: position {} already occupied, \
                                 skipping un-premultiply step",
                                idx + 3
                            );
                        }
                        std::collections::btree_map::Entry::Vacant(slot) => {
                            slot.insert(Box::new(PremultiplyAlpha::new(
                                AlphaState::NonPreMultiplied,
                            )));
                        }
                    }
                } else {
                    log::warn!("No operation found for premultiply at index {idx}")
                }
            })
    }

    map
}

pub enum AvailableEncoders {
    FarbFeld(Box<FarbFeldEncoder>),
    Jpeg(Box<JpegEncoder>),
    JpegXl(Box<JxlEncoder>),
    #[cfg(feature = "mozjpeg")]
    MozJpeg(Box<MozJpegEncoder>),
    #[cfg(feature = "oxipng")]
    OxiPng(Box<OxiPngEncoder>),
    #[cfg(feature = "avif")]
    Avif(Box<AvifEncoder>),
    #[cfg(feature = "webp")]
    Webp(Box<WebPEncoder>),
    Png(Box<PngEncoder>),
    Ppm(Box<PPMEncoder>),
    Qoi(Box<QoiEncoder>),
}

impl AvailableEncoders {
    pub fn to_extension(&self) -> &'static str {
        match self {
            AvailableEncoders::FarbFeld(_) => "ff",
            AvailableEncoders::Jpeg(_) => "jpg",
            AvailableEncoders::JpegXl(_) => "jxl",
            #[cfg(feature = "mozjpeg")]
            AvailableEncoders::MozJpeg(_) => "jpg",
            #[cfg(feature = "oxipng")]
            AvailableEncoders::OxiPng(_) => "png",
            #[cfg(feature = "avif")]
            AvailableEncoders::Avif(_) => "avif",
            #[cfg(feature = "webp")]
            AvailableEncoders::Webp(_) => "webp",
            AvailableEncoders::Png(_) => "png",
            AvailableEncoders::Ppm(_) => "ppm",
            AvailableEncoders::Qoi(_) => "qoi",
        }
    }

    pub fn set_jfif_density(&mut self, density: Option<JfifDensity>) {
        #[cfg(feature = "mozjpeg")]
        {
            let Some(density) = density else {
                return;
            };

            if let AvailableEncoders::MozJpeg(encoder) = self {
                use mozjpeg::{PixelDensity, PixelDensityUnit};

                let unit = match density.unit {
                    0 => PixelDensityUnit::PixelAspectRatio,
                    1 => PixelDensityUnit::Inches,
                    2 => PixelDensityUnit::Centimeters,
                    _ => return,
                };

                encoder.set_pixel_density(PixelDensity {
                    unit,
                    x: density.x_density,
                    y: density.y_density,
                });
            }
        }
        #[cfg(not(feature = "mozjpeg"))]
        {
            let _ = density;
        }
    }

    pub fn encode<T: ZByteWriterTrait>(
        &mut self,
        img: &Image,
        sink: T,
    ) -> Result<usize, ImageErrors> {
        match self {
            AvailableEncoders::FarbFeld(enc) => enc.encode(img, sink),
            AvailableEncoders::Jpeg(enc) => enc.encode(img, sink),
            AvailableEncoders::JpegXl(enc) => enc.encode(img, sink),
            #[cfg(feature = "mozjpeg")]
            AvailableEncoders::MozJpeg(enc) => enc.encode(img, sink),
            #[cfg(feature = "oxipng")]
            AvailableEncoders::OxiPng(enc) => enc.encode(img, sink),
            #[cfg(feature = "avif")]
            AvailableEncoders::Avif(enc) => enc.encode(img, sink),
            #[cfg(feature = "webp")]
            AvailableEncoders::Webp(enc) => enc.encode(img, sink),
            AvailableEncoders::Png(enc) => enc.encode(img, sink),
            AvailableEncoders::Ppm(enc) => enc.encode(img, sink),
            AvailableEncoders::Qoi(enc) => enc.encode(img, sink),
        }
    }
}

pub fn encoder(name: &str, matches: &ArgMatches) -> Result<AvailableEncoders, ImageErrors> {
    match name {
        "farbfeld" => Ok(AvailableEncoders::FarbFeld(
            Box::new(FarbFeldEncoder::new()),
        )),
        "jpeg" => {
            let options = EncoderOptions::default();

            if let Some(quality) = matches.get_one::<u8>("quality") {
                options.set_quality(*quality);
            }

            options.set_jpeg_encode_progressive(matches.get_flag("progressive"));

            Ok(AvailableEncoders::Jpeg(Box::new(
                JpegEncoder::new_with_options(options),
            )))
        }
        "jpeg_xl" => Ok(AvailableEncoders::JpegXl(Box::new(JxlEncoder::new()))),
        #[cfg(feature = "mozjpeg")]
        "mozjpeg" => {
            use mozjpeg::qtable;
            use rimage::codecs::mozjpeg::MozJpegOptions;

            let quality = matches.get_one::<u8>("quality").copied().unwrap_or(75) as f32;
            let chroma_quality = matches
                .get_one::<u8>("chroma_quality")
                .map(|q| *q as f32)
                .unwrap_or(quality);

            let options = MozJpegOptions {
                quality,
                progressive: !matches.get_flag("baseline"),
                optimize_coding: !matches.get_flag("no_optimize_coding"),
                smoothing: matches
                    .get_one::<u8>("smoothing")
                    .copied()
                    .unwrap_or_default(),
                color_space: match matches
                    .get_one::<String>("colorspace")
                    .map(|s| s.as_str())
                    .unwrap_or("ycbcr")
                {
                    "ycbcr" => mozjpeg::ColorSpace::JCS_YCbCr,
                    "rgb" => mozjpeg::ColorSpace::JCS_RGB,
                    "grayscale" => mozjpeg::ColorSpace::JCS_GRAYSCALE,
                    cs => {
                        return Err(ImageErrors::GenericString(format!(
                            "Unsupported mozjpeg colorspace: {cs}",
                        )));
                    }
                },
                trellis_multipass: matches.get_flag("multipass"),
                chroma_subsample: matches.get_one::<u8>("subsample").copied(),

                luma_qtable: match matches.get_one::<String>("qtable") {
                    Some(c) => Some(match c.as_str() {
                        "AhumadaWatsonPeterson" => {
                            qtable::AhumadaWatsonPeterson.scaled(quality, quality)
                        }
                        "AnnexK" => qtable::AnnexK_Luma.scaled(quality, quality),
                        "Flat" => qtable::Flat.scaled(quality, quality),
                        "KleinSilversteinCarney" => {
                            qtable::KleinSilversteinCarney.scaled(quality, quality)
                        }
                        "MSSSIM" => qtable::MSSSIM_Luma.scaled(quality, quality),
                        "NRobidoux" => qtable::NRobidoux.scaled(quality, quality),
                        "PSNRHVS" => qtable::PSNRHVS_Luma.scaled(quality, quality),
                        "PetersonAhumadaWatson" => {
                            qtable::PetersonAhumadaWatson.scaled(quality, quality)
                        }
                        "WatsonTaylorBorthwick" => {
                            qtable::WatsonTaylorBorthwick.scaled(quality, quality)
                        }
                        q => {
                            return Err(ImageErrors::GenericString(
                                format!("Unknown qtable: {q}",),
                            ));
                        }
                    }),
                    None => None,
                },

                chroma_qtable: match matches.get_one::<String>("qtable") {
                    Some(c) => Some(match c.as_str() {
                        "AhumadaWatsonPeterson" => {
                            qtable::AhumadaWatsonPeterson.scaled(chroma_quality, chroma_quality)
                        }
                        "AnnexK" => qtable::AnnexK_Chroma.scaled(chroma_quality, chroma_quality),
                        "Flat" => qtable::Flat.scaled(chroma_quality, chroma_quality),
                        "KleinSilversteinCarney" => {
                            qtable::KleinSilversteinCarney.scaled(chroma_quality, chroma_quality)
                        }
                        "MSSSIM" => qtable::MSSSIM_Chroma.scaled(chroma_quality, chroma_quality),
                        "NRobidoux" => qtable::NRobidoux.scaled(chroma_quality, chroma_quality),
                        "PSNRHVS" => qtable::PSNRHVS_Chroma.scaled(chroma_quality, chroma_quality),
                        "PetersonAhumadaWatson" => {
                            qtable::PetersonAhumadaWatson.scaled(chroma_quality, chroma_quality)
                        }
                        "WatsonTaylorBorthwick" => {
                            qtable::WatsonTaylorBorthwick.scaled(chroma_quality, chroma_quality)
                        }
                        q => {
                            return Err(ImageErrors::GenericString(
                                format!("Unknown qtable: {q}",),
                            ));
                        }
                    }),
                    None => None,
                },
            };

            Ok(AvailableEncoders::MozJpeg(Box::new(
                MozJpegEncoder::new_with_options(options),
            )))
        }
        #[cfg(feature = "oxipng")]
        "oxipng" => {
            use rimage::codecs::oxipng::OxiPngOptions;

            let mut options =
                OxiPngOptions::from_preset(*matches.get_one::<u8>("effort").unwrap_or(&2));

            options.interlace = if matches.get_flag("interlace") {
                Some(true)
            } else {
                None
            };

            Ok(AvailableEncoders::OxiPng(Box::new(
                OxiPngEncoder::new_with_options(options),
            )))
        }
        #[cfg(feature = "avif")]
        "avif" => {
            use rimage::codecs::avif::AvifOptions;

            let options = AvifOptions {
                quality: matches.get_one::<u8>("quality").copied().unwrap_or(50) as f32,
                alpha_quality: matches.get_one::<u8>("alpha_quality").map(|q| *q as f32),
                speed: matches.get_one::<u8>("speed").copied().unwrap_or(6),
                color_space: match matches
                    .get_one::<String>("colorspace")
                    .map(|s| s.as_str())
                    .unwrap_or("ycbcr")
                {
                    "ycbcr" => ravif::ColorModel::YCbCr,
                    "rgb" => ravif::ColorModel::RGB,
                    cs => {
                        return Err(ImageErrors::GenericString(format!(
                            "Unsupported avif colorspace: {cs}",
                        )));
                    }
                },
                alpha_color_mode: match matches
                    .get_one::<String>("alpha_mode")
                    .map(|s| s.as_str())
                    .unwrap_or("UnassociatedClean")
                {
                    "UnassociatedDirty" => ravif::AlphaColorMode::UnassociatedDirty,
                    "UnassociatedClean" => ravif::AlphaColorMode::UnassociatedClean,
                    "Premultiplied" => ravif::AlphaColorMode::Premultiplied,
                    mode => {
                        return Err(ImageErrors::GenericString(format!(
                            "Unsupported avif alpha mode: {mode}",
                        )));
                    }
                },
            };

            Ok(AvailableEncoders::Avif(Box::new(
                AvifEncoder::new_with_options(options),
            )))
        }
        #[cfg(feature = "webp")]
        "webp" => {
            use rimage::codecs::webp::WebPOptions;

            let mut options = WebPOptions::new().map_err(|_| {
                ImageErrors::GenericString(
                    "libwebp encoder configuration failed to initialize".to_string(),
                )
            })?;

            options.quality = matches.get_one::<u8>("quality").copied().unwrap_or(75) as f32;
            options.lossless = matches.get_flag("lossless") as i32;
            options.near_lossless =
                100 - matches.get_one::<u8>("slight_loss").copied().unwrap_or(0) as i32;
            options.exact = matches.get_flag("exact") as i32;

            Ok(AvailableEncoders::Webp(Box::new(
                WebPEncoder::new_with_options(options),
            )))
        }
        "png" => Ok(AvailableEncoders::Png(Box::new(PngEncoder::new()))),
        "ppm" => Ok(AvailableEncoders::Ppm(Box::new(PPMEncoder::new()))),
        "qoi" => Ok(AvailableEncoders::Qoi(Box::new(QoiEncoder::new()))),

        name => Err(ImageErrors::GenericString(format!(
            "Encoder \"{name}\" not found",
        ))),
    }
}

#[cfg(all(test, feature = "resize"))]
mod tests {
    use zune_core::colorspace::ColorSpace;

    use super::*;
    use crate::cli::cli;

    /// Builds the codec subcommand matches the way `main` passes them to [`operations`].
    ///
    /// `farbfeld` is used because it is the one codec that is never feature gated.
    fn matches_from(args: &[&str]) -> ArgMatches {
        cli()
            .get_matches_from(args)
            .subcommand()
            .expect("clap ensures a subcommand is always provided")
            .1
            .clone()
    }

    fn test_image(width: usize, height: usize) -> Image {
        Image::from_fn(width, height, ColorSpace::RGB, |x, y, px: &mut [u8; 4]| {
            px[0] = x as u8;
            px[1] = 0;
            px[2] = y as u8;
        })
    }

    /// Runs the preprocessing pipeline over an image of the given size.
    ///
    /// Reports how many resize operations were queued next to the resulting
    /// dimensions, because "the image already fits" means none were queued at all.
    fn run(resize_args: &[&str], width: usize, height: usize) -> (usize, (usize, usize)) {
        let mut args = vec!["rimage", "farbfeld"];
        args.extend_from_slice(resize_args);
        args.push("image.ff");

        let matches = matches_from(&args);
        let mut img = test_image(width, height);

        let ops = operations(&matches, &img, false);
        let queued = ops.values().filter(|op| op.name() == "fast resize").count();

        for op in ops.values() {
            op.execute(&mut img).unwrap();
        }

        (queued, img.dimensions())
    }

    #[test]
    fn longest_side_anchors_per_orientation() {
        assert_eq!(run(&["--resize", "1000l"], 2000, 1000), (1, (1000, 500)));
        assert_eq!(run(&["--resize", "1000l"], 1000, 2000), (1, (500, 1000)));
        assert_eq!(run(&["--resize", "1000l"], 1600, 1600), (1, (1000, 1000)));
    }

    #[test]
    fn shortest_side_anchors_per_orientation() {
        assert_eq!(run(&["--resize", "500s"], 2000, 1000), (1, (1000, 500)));
        assert_eq!(run(&["--resize", "500s"], 1000, 2000), (1, (500, 1000)));
        assert_eq!(run(&["--resize", "500s"], 1600, 1600), (1, (500, 500)));
    }

    #[test]
    fn side_value_is_skipped_when_the_image_already_fits_exactly() {
        assert_eq!(run(&["--resize", "1000l"], 1000, 500), (0, (1000, 500)));
        assert_eq!(run(&["--resize", "500s"], 1000, 500), (0, (1000, 500)));
    }

    #[test]
    fn no_upscale_leaves_smaller_images_untouched() {
        // The whole point of the flag pair: images already under the target
        // keep their original size instead of being blown up to it.
        assert_eq!(
            run(&["--resize", "1000l", "--no-upscale"], 800, 400),
            (0, (800, 400))
        );

        assert_eq!(
            run(&["--resize", "500s", "--no-upscale"], 800, 400),
            (0, (800, 400))
        );
    }

    #[test]
    fn no_upscale_still_shrinks_larger_images() {
        assert_eq!(
            run(&["--resize", "1000l", "--no-upscale"], 4000, 2000),
            (1, (1000, 500))
        );

        assert_eq!(
            run(&["--resize", "1000l", "--no-upscale"], 2000, 4000),
            (1, (500, 1000))
        );
    }

    #[test]
    fn no_downscale_leaves_larger_images_untouched() {
        assert_eq!(
            run(&["--resize", "1000l", "--no-downscale"], 4000, 2000),
            (0, (4000, 2000))
        );

        assert_eq!(
            run(&["--resize", "500s", "--no-downscale"], 4000, 2000),
            (0, (4000, 2000))
        );
    }

    #[test]
    fn no_downscale_still_enlarges_smaller_images() {
        assert_eq!(
            run(&["--resize", "1000l", "--no-downscale"], 800, 400),
            (1, (1000, 500))
        );

        assert_eq!(
            run(&["--resize", "1000l", "--no-downscale"], 400, 800),
            (1, (500, 1000))
        );
    }

    #[test]
    fn a_mixed_batch_ends_up_with_a_consistent_longest_side() {
        // This is the case `100w` cannot express: with a fixed width anchor the
        // portrait images below would come out far taller than the landscape
        // ones are wide.
        for (width, height) in [(4000, 3000), (3000, 4000), (2000, 2000), (1200, 900)] {
            let (_, (new_width, new_height)) = run(&["--resize", "1000l"], width, height);

            assert_eq!(
                new_width.max(new_height),
                1000,
                "{width}x{height} gave {new_width}x{new_height}"
            );
        }
    }

    #[test]
    fn a_mixed_batch_ends_up_with_a_consistent_shortest_side() {
        for (width, height) in [(4000, 3000), (3000, 4000), (2000, 2000), (1200, 900)] {
            let (_, (new_width, new_height)) = run(&["--resize", "500s"], width, height);

            assert_eq!(
                new_width.min(new_height),
                500,
                "{width}x{height} gave {new_width}x{new_height}"
            );
        }
    }

    #[test]
    fn shrink_only_batch_touches_only_oversized_images() {
        // 1000l with --no-upscale is the shrink only mode from the feature
        // request: everything ends up at or below 1000px, and anything that was
        // already small keeps its exact original size.
        let batch = [
            ((4000, 3000), (1000, 750)),
            ((3000, 4000), (750, 1000)),
            ((800, 600), (800, 600)),
            ((1000, 1000), (1000, 1000)),
        ];

        for ((width, height), expected) in batch {
            let (_, dimensions) = run(&["--resize", "1000l", "--no-upscale"], width, height);

            assert_eq!(dimensions, expected, "failed on {width}x{height}");
        }
    }

    #[test]
    fn reduce_only_is_an_alias_for_no_upscale() {
        for (width, height) in [(4000, 2000), (800, 400), (1000, 500)] {
            assert_eq!(
                run(&["--resize", "1000l", "--reduce-only"], width, height),
                run(&["--resize", "1000l", "--no-upscale"], width, height),
                "--reduce-only diverged from --no-upscale on {width}x{height}"
            );
        }
    }

    #[test]
    fn enlarge_only_is_an_alias_for_no_downscale() {
        for (width, height) in [(4000, 2000), (800, 400), (1000, 500)] {
            assert_eq!(
                run(&["--resize", "1000l", "--enlarge-only"], width, height),
                run(&["--resize", "1000l", "--no-downscale"], width, height),
                "--enlarge-only diverged from --no-downscale on {width}x{height}"
            );
        }
    }

    #[test]
    fn reduce_only_and_enlarge_only_pick_the_direction_their_names_promise() {
        // Guards against the aliases being wired to the opposite flag, which is
        // the whole risk of naming a double negative.
        assert_eq!(
            run(&["--resize", "1000l", "--reduce-only"], 4000, 2000),
            (1, (1000, 500))
        );
        assert_eq!(
            run(&["--resize", "1000l", "--reduce-only"], 800, 400),
            (0, (800, 400))
        );

        assert_eq!(
            run(&["--resize", "1000l", "--enlarge-only"], 800, 400),
            (1, (1000, 500))
        );
        assert_eq!(
            run(&["--resize", "1000l", "--enlarge-only"], 4000, 2000),
            (0, (4000, 2000))
        );
    }

    #[test]
    fn width_anchor_with_reduce_caps_the_width_only() {
        // Capping one dimension however long the other one is does not need a
        // side value at all, the existing width anchor already covers it. The
        // height follows the aspect ratio, and images at or under the cap keep
        // their original size.
        assert_eq!(
            run(&["--resize", "400w", "--reduce-only"], 200, 100),
            (0, (200, 100))
        );
        assert_eq!(
            run(&["--resize", "400w", "--reduce-only"], 400, 2000),
            (0, (400, 2000))
        );
        assert_eq!(
            run(&["--resize", "400w", "--reduce-only"], 800, 900),
            (1, (400, 450))
        );
    }

    #[test]
    fn disabling_both_directions_skips_every_resize() {
        // Neither flag wins over the other, they both apply, which leaves no
        // direction to resize in. The order they are passed does not matter.
        for args in [
            ["--resize", "1000l", "--no-upscale", "--no-downscale"],
            ["--resize", "1000l", "--no-downscale", "--no-upscale"],
            ["--resize", "1000l", "--reduce-only", "--enlarge-only"],
        ] {
            assert_eq!(run(&args, 4000, 2000), (0, (4000, 2000)));
            assert_eq!(run(&args, 800, 400), (0, (800, 400)));
            assert_eq!(run(&args, 1000, 500), (0, (1000, 500)));
        }
    }

    #[test]
    fn chained_resize_values_compose() {
        // Each value maps the size the previous resize left behind, so the
        // chain applies in order instead of every value mapping the source.
        assert_eq!(
            run(&["--resize", "100x400", "--resize", "200s"], 800, 400),
            (2, (200, 800))
        );

        assert_eq!(
            run(&["--resize", "1000l", "--resize", "50%"], 800, 400),
            (2, (500, 250))
        );

        // The first resize already lands on the target the second one maps to,
        // so the second is skipped as "already fits" instead of running twice.
        assert_eq!(
            run(&["--resize", "1000l", "--resize", "500s"], 800, 400),
            (1, (1000, 500))
        );

        // A skipped resize leaves the current size unchanged, so the next value
        // maps the size the resize actually ran on.
        assert_eq!(
            run(
                &["--resize", "2000l", "--resize", "50%", "--no-upscale"],
                800,
                400
            ),
            (1, (400, 200))
        );
    }

    #[test]
    fn side_values_compose_with_the_filter_flag() {
        assert_eq!(
            run(&["--resize", "1000l", "--filter", "nearest"], 2000, 1000),
            (1, (1000, 500))
        );
    }

    #[test]
    fn skip_resize_skips_only_leading_svg_steps() {
        let matches = matches_from(&[
            "rimage",
            "farbfeld",
            "--resize",
            "@2",
            "--quantization",
            "80",
            "--resize",
            "50%",
            "image.ff",
        ]);

        let img = test_image(200, 100);
        let ops = operations(&matches, &img, true);

        let resize_indices: Vec<usize> = ops
            .iter()
            .filter(|(_, op)| op.name() == "fast resize")
            .map(|(idx, _)| *idx)
            .collect();

        let expected: Vec<usize> = matches
            .indices_of("resize")
            .into_iter()
            .flatten()
            .skip(1)
            .collect();
        assert_eq!(resize_indices, expected);
    }

    #[cfg(feature = "svg")]
    mod svg_target_size_tests {
        use zune_image::errors::ImageErrors;

        use super::super::svg_target_size;
        use super::matches_from;

        fn target_size(args: &[&str]) -> Result<Option<(u32, u32)>, ImageErrors> {
            let mut file_args = vec!["rimage", "farbfeld"];
            file_args.extend_from_slice(args);
            file_args.push("tests/files/svg/rect.svg");

            let matches = matches_from(&file_args);
            svg_target_size(&matches, (100, 50))
        }

        #[test]
        fn multiplier_uses_vector_render_target() {
            assert_eq!(target_size(&["--resize", "@2"]).unwrap(), Some((200, 100)));
        }

        #[test]
        fn chained_resize_composes_for_svg() {
            assert_eq!(
                target_size(&["--resize", "@2", "--resize", "50%"]).unwrap(),
                Some((100, 50))
            );
        }

        #[test]
        fn intrinsic_size_returns_no_target() {
            assert_eq!(target_size(&["--resize", "100x50"]).unwrap(), None);
        }

        #[test]
        fn no_upscale_skips_growth_for_svg() {
            assert_eq!(
                target_size(&["--resize", "200l", "--no-upscale"]).unwrap(),
                None
            );
        }

        #[test]
        fn resize_after_quantization_is_not_preapplied() {
            assert_eq!(
                target_size(&["--quantization", "80", "--resize", "64x64"]).unwrap(),
                None
            );
        }

        #[test]
        fn premultiply_before_resize_is_not_preapplied() {
            assert_eq!(
                target_size(&["--premultiply", "--resize", "64x64"]).unwrap(),
                None
            );
        }

        #[test]
        fn longest_side_upscales_svg_vectorly() {
            assert_eq!(
                target_size(&["--resize", "200l"]).unwrap(),
                Some((200, 100))
            );
        }

        #[test]
        fn filter_is_accepted_but_ignored_for_svg_vector_resize() {
            assert_eq!(
                target_size(&["--resize", "@2", "--filter", "nearest"]).unwrap(),
                Some((200, 100))
            );
        }

        // `usize` cannot exceed `u32::MAX` on 32-bit targets, so the
        // overflow path this test guards only exists on 64-bit platforms.
        #[cfg(target_pointer_width = "64")]
        #[test]
        fn oversized_dimensions_are_rejected() {
            assert!(target_size(&["--resize", "4294967396x4294967396"]).is_err());
        }
    }
}

#[cfg(all(test, feature = "limits"))]
mod limit_tests {
    use super::*;
    use crate::cli::cli;

    /// Builds the codec subcommand matches the way `main` passes them to
    /// [`decode`]. Local to this module because the shared helper lives behind
    /// the `resize` feature, and the limit checks must be testable without it.
    fn matches_from(args: &[&str]) -> ArgMatches {
        cli()
            .get_matches_from(args)
            .subcommand()
            .expect("clap ensures a subcommand is always provided")
            .1
            .clone()
    }

    /// A JPEG header probe must recover the real dimensions from an ordinary
    /// file, or the pre-check would silently never fire.
    #[test]
    fn a_jpeg_header_probe_reads_the_real_dimensions() {
        let file = File::open("tests/files/jpg/f1t.jpg").unwrap();

        let probed = probe_dimensions(file, rimage::limits::ImageFormatId::Jpeg);

        let (width, height) = probed.expect("the fixture's header must be readable");
        assert!(width > 0 && height > 0, "got {width}x{height}");
    }

    /// A format with no published limit and no decoder ceiling is not probed;
    /// its only bound is the memory budget, which the decoder applies to the
    /// buffer it allocates.
    #[test]
    fn formats_without_a_side_limit_are_not_probed() {
        let file = File::open("tests/files/jpg/f1t.jpg").unwrap();

        assert!(
            probe_dimensions(file, rimage::limits::ImageFormatId::Tiff).is_none(),
            "format identity alone must not imply a probe; TIFF is probed by \
             its own reader but a JPEG body is not a TIFF"
        );
    }

    /// PNG dimensions are in the `IHDR` chunk, which the spec requires to come
    /// first, so a short prefix settles them.
    #[test]
    fn a_png_header_probe_reads_the_real_dimensions() {
        let file = File::open("tests/files/png/f1trgba.png").unwrap();

        let (width, height) = probe_dimensions(file, rimage::limits::ImageFormatId::Png)
            .expect("the fixture's IHDR must be readable");

        assert!(width > 0 && height > 0, "got {width}x{height}");
    }

    /// A PNG whose first chunk is not `IHDR` is malformed, and its dimensions
    /// must not be read from wherever a 16-byte window happens to land.
    #[test]
    fn a_png_without_a_leading_ihdr_is_not_probed() {
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(b"IDAT");
        data.extend_from_slice(&[0; 16]);

        assert!(png_dimensions(&data).is_none());
    }

    /// A file that does not start with the PNG signature is not a PNG, whatever
    /// else it holds.
    #[test]
    fn a_file_that_is_not_a_png_is_not_probed() {
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&[0; 16]);

        assert!(png_dimensions(&data).is_none());
    }

    /// WebP has a published 16383-per-side limit, so its header must be
    /// readable without decoding. The bitstream-features probe is what makes
    /// that possible; if it ever stops working the check silently goes dead.
    #[cfg(feature = "webp")]
    #[test]
    fn a_webp_header_probe_reads_the_real_dimensions() {
        let file = File::open("tests/files/webp/f1t.webp").unwrap();

        let (width, height) = probe_dimensions(file, rimage::limits::ImageFormatId::WebP)
            .expect("the fixture's header must be readable");

        assert!(width > 0 && height > 0, "got {width}x{height}");
    }

    /// A file too short to contain a frame header must be reported as
    /// unprobeable rather than as an error, so decoding still gets its chance.
    #[test]
    fn a_truncated_header_is_not_an_error() {
        let truncated = std::io::Cursor::new(vec![0xFF, 0xD8, 0xFF]);

        assert!(probe_dimensions(truncated, rimage::limits::ImageFormatId::Jpeg).is_none());
    }

    /// AVIF keeps its size in an ISO-BMFF property box rather than a header, so
    /// the box walk is the only thing standing between an oversized file and
    /// the decoder. A wrong answer here is worse than none, so the dimensions
    /// are compared against the values `ispe` actually declares.
    #[test]
    fn an_avif_probe_reads_the_dimensions_from_the_ispe_box() {
        let file = File::open("tests/files/avif/f1t.avif").unwrap();

        let (width, height) = probe_dimensions(file, rimage::limits::ImageFormatId::Avif)
            .expect("the fixture's ispe box must be reachable");

        // The fixture is a real 48x80 image; these are the values its `ispe`
        // box carries.
        assert_eq!((width, height), (48, 80));
    }

    /// A byte sequence that merely looks like `ispe` must not be mistaken for
    /// the property box. The walk keys off declared box sizes, so a file with
    /// no `meta`/`iprp`/`ipco` chain yields nothing rather than a bogus size.
    #[test]
    fn an_avif_file_without_the_property_chain_is_not_probed() {
        // A well-formed `ftyp` followed by bytes that spell `ispe` where no
        // property box can legally appear.
        let mut data = Vec::new();
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"avif");
        data.extend_from_slice(&[0; 8]);
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(b"ispe");
        data.extend_from_slice(&[0; 12]);

        assert!(avif_dimensions(&data).is_none());
    }

    /// TIFF stores its dimensions in an IFD reached through an offset in the
    /// header, so the tag walk has to follow that offset rather than scan.
    #[test]
    fn a_tiff_probe_reads_the_dimensions_from_the_ifd() {
        let file = File::open("tests/files/tiff/f1t.tif").unwrap();

        let (width, height) = probe_dimensions(file, rimage::limits::ImageFormatId::Tiff)
            .expect("the fixture's IFD must be readable");

        assert!(width > 0 && height > 0, "got {width}x{height}");
    }

    /// Build a minimal TIFF containing only the two dimension tags.
    ///
    /// `value_type` is the TIFF type code, so the same builder covers both the
    /// `SHORT` and `LONG` encodings the specification allows here.
    fn tiff_with_dimensions(order: &[u8; 2], value_type: u16, width: u32, height: u32) -> Vec<u8> {
        let big = order == b"MM";
        let mut file = Vec::new();
        file.extend_from_slice(order);

        let push_u16 = |file: &mut Vec<u8>, value: u16| {
            let bytes = if big {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            };
            file.extend_from_slice(&bytes);
        };
        let push_u32 = |file: &mut Vec<u8>, value: u32| {
            let bytes = if big {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            };
            file.extend_from_slice(&bytes);
        };

        push_u16(&mut file, 42);
        push_u32(&mut file, 8);

        // Two IFD entries, in tag order.
        push_u16(&mut file, 2);
        for (tag, value) in [(0x0100u16, width), (0x0101, height)] {
            push_u16(&mut file, tag);
            push_u16(&mut file, value_type);
            push_u32(&mut file, 1);
            // A `SHORT` is stored left-justified and padded; a `LONG` fills the
            // whole value field.
            if value_type == 3 {
                push_u16(&mut file, value as u16);
                push_u16(&mut file, 0);
            } else {
                push_u32(&mut file, value);
            }
        }

        file
    }

    /// Both TIFF byte orders are in the wild, and both must be accepted. The
    /// same tag values are written little-endian and big-endian here.
    #[test]
    fn a_tiff_probe_handles_both_byte_orders() {
        assert_eq!(
            tiff_dimensions(&tiff_with_dimensions(b"II", 4, 132_778, 5_000)),
            Some((132_778, 5_000)),
            "little-endian TIFF must be readable"
        );
        assert_eq!(
            tiff_dimensions(&tiff_with_dimensions(b"MM", 4, 132_778, 5_000)),
            Some((132_778, 5_000)),
            "big-endian TIFF must be readable"
        );
    }

    /// `SHORT` is the common encoding for small dimensions and is stored
    /// left-justified in the four-byte value field, which is the one place a
    /// big-endian file differs from a little-endian one.
    #[test]
    fn a_tiff_probe_reads_short_values() {
        assert_eq!(
            tiff_dimensions(&tiff_with_dimensions(b"II", 3, 640, 480)),
            Some((640, 480)),
            "little-endian SHORT must be readable"
        );
        assert_eq!(
            tiff_dimensions(&tiff_with_dimensions(b"MM", 3, 640, 480)),
            Some((640, 480)),
            "big-endian SHORT must be readable"
        );
    }

    /// A file whose magic number is wrong is not a TIFF, whatever else it
    /// contains, and must not be reported as one.
    #[test]
    fn a_file_that_is_not_a_tiff_is_not_probed() {
        let mut file = Vec::new();
        file.extend_from_slice(b"XX");
        file.extend_from_slice(&42u16.to_le_bytes());
        file.extend_from_slice(&8u32.to_le_bytes());
        file.extend_from_slice(&[0; 32]);

        assert!(tiff_dimensions(&file).is_none());
    }

    /// An ordinary image on this machine must pass the pre-check, so the limit
    /// does not reject files the program is expected to handle.
    #[test]
    fn an_ordinary_image_passes_the_pre_check() {
        let matches = matches_from(&["rimage", "mozjpeg", "tests/files/jpg/f1t.jpg"]);

        let output = rimage::limits::ImageFormatId::Jpeg;

        check_input_limits(Path::new("tests/files/jpg/f1t.jpg"), &matches, output, 1)
            .expect("an ordinary fixture must not be rejected");
    }

    /// A path that does not exist is left for the decoder to report, rather
    /// than being turned into a size error here.
    #[test]
    fn a_missing_file_is_not_a_size_error() {
        let matches = matches_from(&["rimage", "mozjpeg", "tests/files/does-not-exist.jpg"]);

        let output = rimage::limits::ImageFormatId::Jpeg;

        assert!(
            check_input_limits(
                Path::new("tests/files/does-not-exist.jpg"),
                &matches,
                output,
                1
            )
            .is_ok()
        );
    }

    /// The ceiling the screen applies has to move with the concurrency it is
    /// handed, because that number is the budget's divisor. It is asserted as
    /// a strict ordering rather than on absolute pixel counts, which would
    /// depend on how much memory the host happens to have free.
    #[test]
    fn a_higher_concurrency_lowers_the_pixel_ceiling() {
        let output = rimage::limits::ImageFormatId::Jpeg;

        let one_at_a_time = rimage::limits::LimitSet::for_input(
            rimage::limits::ImageFormatId::Jpeg,
            zune_core::bit_depth::BitDepth::Eight,
            zune_core::colorspace::ColorSpace::RGBA,
            &rimage::limits::SystemBudget::probe(1),
            rimage::limits::PipelineCost::for_conversion(
                rimage::limits::ImageFormatId::Jpeg,
                output,
            ),
        );
        let ten_at_a_time = rimage::limits::LimitSet::for_input(
            rimage::limits::ImageFormatId::Jpeg,
            zune_core::bit_depth::BitDepth::Eight,
            zune_core::colorspace::ColorSpace::RGBA,
            &rimage::limits::SystemBudget::probe(10),
            rimage::limits::PipelineCost::for_conversion(
                rimage::limits::ImageFormatId::Jpeg,
                output,
            ),
        );

        // A failed probe returns the same fixed fallback for every concurrency,
        // so the two pixel ceilings come out equal and there is nothing to
        // compare; only assert against a figure the host actually reported.
        if !rimage::limits::SystemBudget::probe(1).is_probed() {
            return;
        }

        if one_at_a_time.binding == rimage::limits::Binding::Memory {
            assert!(one_at_a_time.max_pixels > ten_at_a_time.max_pixels);
        }
    }
}
