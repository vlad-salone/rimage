use std::{io::Read, marker::PhantomData};

use dav1d::{
    Decoder, Error as Dav1dError, Picture, PixelLayout, PlanarImageComponent,
    pixel::{MatrixCoefficients, YUVRange as Dav1dYuvRange},
};
use yuvutils_rs::{
    YuvGrayImage, YuvPlanarImage, YuvRange, YuvStandardMatrix, ycgco420_to_rgba, ycgco422_to_rgba,
    ycgco444_to_rgba, yuv400_to_rgba, yuv420_to_rgba, yuv422_to_rgba, yuv444_to_rgba,
};
use zune_core::colorspace::ColorSpace;
use zune_image::{errors::ImageErrors, image::Image, traits::DecoderTrait};

/// Checks whether `data` looks like an AVIF file.
///
/// An ISO-BMFF file must start with an `ftyp` box (ISO 14496-12 § 4.3.1) that
/// carries the `avif` or `avis` brand either as the major brand or somewhere in
/// the compatible brands list.
pub fn is_avif(data: &[u8]) -> bool {
    if data.len() < 12 || &data[4..8] != b"ftyp" {
        return false;
    }

    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let end = if size == 0 {
        data.len()
    } else {
        size.min(data.len())
    };

    let brand = |b: &[u8]| b == b"avif" || b == b"avis";

    if brand(&data[8..12]) {
        return true;
    }

    // compatible brands start after the 4-byte minor version
    end >= 16
        && data[16..end]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|arg0: &[u8; 4]| brand(arg0))
}

/// A AVIF decoder that decodes the AV1 bitstream through `dav1d`
pub struct AvifDecoder<R: Read> {
    inner: Vec<u8>,
    dimensions: Option<(usize, usize)>,
    phantom: PhantomData<R>,
}

impl<R: Read> AvifDecoder<R> {
    /// Create a new avif decoder that reads data from `source`
    pub fn try_new(mut source: R) -> Result<AvifDecoder<R>, ImageErrors> {
        let mut buf = Vec::new();
        source.read_to_end(&mut buf)?;

        Ok(AvifDecoder {
            inner: buf,
            dimensions: None,
            phantom: PhantomData,
        })
    }
}

impl<R> DecoderTrait for AvifDecoder<R>
where
    R: Read,
{
    fn decode(&mut self) -> Result<Image, ImageErrors> {
        let parsed = avif_parse::read_avif(&mut self.inner.as_slice()).map_err(decode_error)?;

        let color = decode_av1_stream(&parsed.primary_item)?;
        let alpha = parsed
            .alpha_item
            .as_deref()
            .filter(|stream| !stream.is_empty())
            .map(decode_av1_stream)
            .transpose()?;

        let rgba = picture_to_rgba(&color, alpha.as_ref(), parsed.premultiplied_alpha)?;

        let (width, height) = (color.width() as usize, color.height() as usize);
        self.dimensions = Some((width, height));

        Ok(Image::from_u8(&rgba, width, height, ColorSpace::RGBA))
    }

    fn dimensions(&self) -> Option<(usize, usize)> {
        self.dimensions
    }

    fn out_colorspace(&self) -> ColorSpace {
        ColorSpace::RGBA
    }

    fn name(&self) -> &'static str {
        "avif-decoder (dav1d)"
    }
}

fn decode_error(err: impl std::fmt::Display) -> ImageErrors {
    ImageErrors::ImageDecodeErrors(format!("avif: {err}"))
}

/// Decodes a still-picture AV1 bitstream and returns its frame.
///
/// Still AVIF carries a single frame, so frame threading is disabled through
/// `max_frame_delay`: it would otherwise hold the picture back until an
/// end-of-stream drain, and `dav1d_flush` only discards state instead of
/// draining it.
fn decode_av1_stream(data: &[u8]) -> Result<Picture, ImageErrors> {
    let mut settings = dav1d::Settings::new();
    settings.set_max_frame_delay(1);

    let mut decoder = Decoder::with_settings(&settings).map_err(decode_error)?;

    match decoder.send_data(data.to_vec(), None, None, None) {
        // `Again` only means decoded frames are pending retrieval, not rejection
        Ok(()) | Err(Dav1dError::Again) => {}
        Err(err) => return Err(decode_error(err)),
    }

    let mut pending_pushed = false;
    loop {
        match decoder.get_picture() {
            Ok(picture) => return Ok(picture),
            Err(Dav1dError::Again) => {
                if pending_pushed {
                    return Err(ImageErrors::ImageDecodeErrors(
                        "avif: the bitstream contains no frame".into(),
                    ));
                }
                decoder.send_pending_data().map_err(decode_error)?;
                pending_pushed = true;
            }
            Err(err) => return Err(decode_error(err)),
        }
    }
}

fn picture_to_rgba(
    color: &Picture,
    alpha: Option<&Picture>,
    premultiplied: bool,
) -> Result<Vec<u8>, ImageErrors> {
    let (width, height) = (color.width() as usize, color.height() as usize);

    if let Some(alpha) = alpha
        && (alpha.width() as usize != width || alpha.height() as usize != height)
    {
        return Err(ImageErrors::ImageDecodeErrors(
            "avif: alpha item dimensions do not match the color item".into(),
        ));
    }

    let rgba_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            ImageErrors::ImageDecodeErrors(
                "avif: decoded dimensions overflow the output buffer size".into(),
            )
        })?;

    let mut rgba = vec![0u8; rgba_len];

    if color.matrix_coefficients() == MatrixCoefficients::Identity {
        identity_planes_to_rgba(color, &mut rgba)?;
    } else {
        color_planes_to_rgba(color, &mut rgba)?;
    }

    if let Some(alpha) = alpha {
        merge_alpha_plane(&mut rgba, alpha, width, height)?;
    }

    if premultiplied {
        unpremultiply_rgba(&mut rgba);
    }

    Ok(rgba)
}

enum ColorTransform {
    /// YCbCr with the ITU-R kr/kb weights of the bitstream's matrix, plus the
    /// matching yuvutils preset for the 8-bit path.
    Ycbcr {
        kr: f32,
        kb: f32,
        standard: YuvStandardMatrix,
    },
    /// YCgCo (H.273 matrix 8); full range only.
    Ycgco,
}

fn color_transform(coefficients: MatrixCoefficients) -> Result<ColorTransform, ImageErrors> {
    match coefficients {
        MatrixCoefficients::BT709 => Ok(ColorTransform::Ycbcr {
            kr: 0.2126,
            kb: 0.0722,
            standard: YuvStandardMatrix::Bt709,
        }),
        MatrixCoefficients::BT470M => Ok(ColorTransform::Ycbcr {
            kr: 0.30,
            kb: 0.11,
            standard: YuvStandardMatrix::Custom(0.30, 0.11),
        }),
        // BT.470BG and ST-170M are both BT.601; libavif also defaults to BT.601
        // when the bitstream leaves the matrix unspecified
        MatrixCoefficients::BT470BG
        | MatrixCoefficients::ST170M
        | MatrixCoefficients::Unspecified => Ok(ColorTransform::Ycbcr {
            kr: 0.299,
            kb: 0.114,
            standard: YuvStandardMatrix::Bt601,
        }),
        MatrixCoefficients::ST240M => Ok(ColorTransform::Ycbcr {
            kr: 0.212,
            kb: 0.087,
            standard: YuvStandardMatrix::Smpte240,
        }),
        MatrixCoefficients::BT2020ConstantLuminance => Err(decode_error(
            "unsupported matrix coefficients (BT.2020 constant luminance)",
        )),
        MatrixCoefficients::BT2020NonConstantLuminance => Ok(ColorTransform::Ycbcr {
            kr: 0.2627,
            kb: 0.0593,
            standard: YuvStandardMatrix::Bt2020,
        }),
        MatrixCoefficients::YCgCo => Ok(ColorTransform::Ycgco),
        other => Err(decode_error(format!(
            "unsupported matrix coefficients ({other})"
        ))),
    }
}

fn color_planes_to_rgba(color: &Picture, rgba: &mut [u8]) -> Result<(), ImageErrors> {
    let (width, height) = (color.width() as usize, color.height() as usize);
    let range = match color.color_range() {
        Dav1dYuvRange::Full => YuvRange::Full,
        _ => YuvRange::Limited,
    };
    let transform = color_transform(color.matrix_coefficients())?;
    let layout = color.pixel_layout();

    if layout == PixelLayout::I400 {
        return monochrome_to_rgba(color, rgba, range);
    }

    if color.bit_depth() == 8 {
        let image = YuvPlanarImage {
            y_plane: &color.plane(PlanarImageComponent::Y),
            y_stride: color.stride(PlanarImageComponent::Y),
            u_plane: &color.plane(PlanarImageComponent::U),
            u_stride: color.stride(PlanarImageComponent::U),
            v_plane: &color.plane(PlanarImageComponent::V),
            v_stride: color.stride(PlanarImageComponent::V),
            width: width as u32,
            height: height as u32,
        };

        match (transform, layout) {
            (ColorTransform::Ycbcr { standard, .. }, PixelLayout::I420) => {
                yuv420_to_rgba(&image, rgba, rgba_stride(width), range, standard)
            }
            (ColorTransform::Ycbcr { standard, .. }, PixelLayout::I422) => {
                yuv422_to_rgba(&image, rgba, rgba_stride(width), range, standard)
            }
            (ColorTransform::Ycbcr { standard, .. }, PixelLayout::I444) => {
                yuv444_to_rgba(&image, rgba, rgba_stride(width), range, standard)
            }
            (ColorTransform::Ycgco, PixelLayout::I420) => {
                ycgco420_to_rgba(&image, rgba, rgba_stride(width), range)
            }
            (ColorTransform::Ycgco, PixelLayout::I422) => {
                ycgco422_to_rgba(&image, rgba, rgba_stride(width), range)
            }
            (ColorTransform::Ycgco, PixelLayout::I444) => {
                ycgco444_to_rgba(&image, rgba, rgba_stride(width), range)
            }
            _ => unreachable!("layout is not monochrome here"),
        }
        .map_err(decode_error)
    } else {
        // `Plane` owns an `Arc` clone of the picture, so bind the planes here
        // and hand `Planes` slices of them
        let y_plane = color.plane(PlanarImageComponent::Y);
        let u_plane = color.plane(PlanarImageComponent::U);
        let v_plane = color.plane(PlanarImageComponent::V);
        let view = Planes {
            y: &y_plane,
            y_stride: color.stride(PlanarImageComponent::Y) as usize,
            u: &u_plane,
            u_stride: color.stride(PlanarImageComponent::U) as usize,
            v: &v_plane,
            v_stride: color.stride(PlanarImageComponent::V) as usize,
            width,
            height,
            layout,
        };
        high_depth_ycbcr_to_rgba(
            &view,
            color
                .bits_per_component()
                .map(|bits| bits.0.clamp(8, 16))
                .unwrap_or(8),
            color.color_range() == Dav1dYuvRange::Full,
            &transform,
            rgba,
        );
        Ok(())
    }
}

/// Monochrome pictures only carry luma; gray expansion never uses the matrix.
fn monochrome_to_rgba(
    color: &Picture,
    rgba: &mut [u8],
    range: YuvRange,
) -> Result<(), ImageErrors> {
    let (width, height) = (color.width() as usize, color.height() as usize);

    if color.bit_depth() == 8 {
        let gray = YuvGrayImage {
            y_plane: &color.plane(PlanarImageComponent::Y),
            y_stride: color.stride(PlanarImageComponent::Y),
            width: width as u32,
            height: height as u32,
        };

        yuv400_to_rgba(
            &gray,
            rgba,
            rgba_stride(width),
            range,
            YuvStandardMatrix::Bt601,
        )
        .map_err(decode_error)
    } else {
        let used = used_bits(color)?;
        let full_range = color.color_range() == Dav1dYuvRange::Full;
        let plane = &color.plane(PlanarImageComponent::Y);
        let stride = color.stride(PlanarImageComponent::Y) as usize;

        check_plane(plane, stride, height, width * 2)?;

        for row in 0..height {
            let source = &plane[row * stride..][..width * 2];
            let out = &mut rgba[row * width * 4..][..width * 4];
            for (x, bytes) in source.as_chunks::<2>().0.iter().enumerate() {
                let sample = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
                // clamp: lossy bitstreams can undershoot the limited range
                let gray = expand_luma(sample, used, full_range).clamp(0.0, 255.0) as u8;
                out[x * 4] = gray;
                out[x * 4 + 1] = gray;
                out[x * 4 + 2] = gray;
                out[x * 4 + 3] = 255;
            }
        }
        Ok(())
    }
}

/// AV1 identity matrix carries RGB directly in the planes: Y=G, U=B, V=R.
fn identity_planes_to_rgba(color: &Picture, rgba: &mut [u8]) -> Result<(), ImageErrors> {
    let (width, height) = (color.width() as usize, color.height() as usize);
    let g_plane = &color.plane(PlanarImageComponent::Y);
    let g_stride = color.stride(PlanarImageComponent::Y) as usize;
    let b_plane = &color.plane(PlanarImageComponent::U);
    let b_stride = color.stride(PlanarImageComponent::U) as usize;
    let r_plane = &color.plane(PlanarImageComponent::V);
    let r_stride = color.stride(PlanarImageComponent::V) as usize;

    let bytes_per_sample = if color.bit_depth() == 8 { 1 } else { 2 };
    for (plane, stride) in [
        (&r_plane[..], r_stride),
        (&g_plane[..], g_stride),
        (&b_plane[..], b_stride),
    ] {
        check_plane(plane, stride, height, width * bytes_per_sample)?;
    }

    if color.bit_depth() == 8 {
        for y in 0..height {
            let row = &mut rgba[y * width * 4..][..width * 4];
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                px[0] = r_plane[y * r_stride + x];
                px[1] = g_plane[y * g_stride + x];
                px[2] = b_plane[y * b_stride + x];
                px[3] = 255;
            }
        }
    } else {
        let used = used_bits(color).unwrap_or(16);
        for y in 0..height {
            let row = &mut rgba[y * width * 4..][..width * 4];
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                px[0] = sample_u8(&r_plane[y * r_stride + x * 2..], used);
                px[1] = sample_u8(&g_plane[y * g_stride + x * 2..], used);
                px[2] = sample_u8(&b_plane[y * b_stride + x * 2..], used);
                px[3] = 255;
            }
        }
    }

    Ok(())
}

/// Copies the alpha stream's luma plane into the RGBA alpha channel.
fn merge_alpha_plane(
    rgba: &mut [u8],
    alpha: &Picture,
    width: usize,
    height: usize,
) -> Result<(), ImageErrors> {
    let plane = &alpha.plane(PlanarImageComponent::Y);
    let stride = alpha.stride(PlanarImageComponent::Y) as usize;

    let bytes_per_sample = if alpha.bit_depth() == 8 { 1 } else { 2 };
    check_plane(plane, stride, height, width * bytes_per_sample)?;

    if alpha.bit_depth() == 8 {
        for y in 0..height {
            let source = &plane[y * stride..][..width];
            let row = &mut rgba[y * width * 4..][..width * 4];
            for (x, &a) in source.iter().enumerate() {
                row[x * 4 + 3] = a;
            }
        }
    } else {
        let used = used_bits(alpha).unwrap_or(16);
        for y in 0..height {
            let source = &plane[y * stride..][..width * 2];
            let row = &mut rgba[y * width * 4..][..width * 4];
            for (x, bytes) in source.as_chunks::<2>().0.iter().enumerate() {
                row[x * 4 + 3] = sample_u8(bytes, used);
            }
        }
    }

    Ok(())
}

/// Verify a plane holds `height` rows of `row_bytes` at `stride`, so the
/// conversion loops can index it without per-pixel bounds checks.
///
/// dav1d promises a stride of at least the row width, but a short plane is
/// not worth a panic in a worker thread, where the release profile's
/// `panic = "abort"` would take down the whole process.
fn check_plane(
    plane: &[u8],
    stride: usize,
    height: usize,
    row_bytes: usize,
) -> Result<(), ImageErrors> {
    let needed = height
        .checked_sub(1)
        .map_or(Some(0), |last_row| last_row.checked_mul(stride))
        .and_then(|offset| offset.checked_add(row_bytes));

    match needed {
        Some(needed) if plane.len() >= needed => Ok(()),
        _ => Err(decode_error(
            "decoded plane is smaller than its dimensions imply",
        )),
    }
}

/// Recovers straight alpha from premultiplied color (MIAF § 7.3.5.2).
fn unpremultiply_rgba(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// Raw planar slices of a picture, valid as long as the borrowed `Plane`s.
struct Planes<'a> {
    y: &'a [u8],
    y_stride: usize,
    u: &'a [u8],
    u_stride: usize,
    v: &'a [u8],
    v_stride: usize,
    width: usize,
    height: usize,
    layout: PixelLayout,
}

impl Planes<'_> {
    fn sample(&self, component: Sample, x: usize, y: usize) -> u16 {
        let (plane, stride) = match component {
            Sample::Y => (self.y, self.y_stride),
            Sample::U => (self.u, self.u_stride),
            Sample::V => (self.v, self.v_stride),
        };
        let offset = y * stride + x * 2;
        // Bounds-check the 2-byte sample: if the plane is shorter than
        // expected (corrupted bitstream or mismatched stride), return 0
        // rather than panicking across the FFI boundary.
        if offset + 2 > plane.len() {
            return 0;
        }
        u16::from_le_bytes([plane[offset], plane[offset + 1]])
    }
}

enum Sample {
    Y,
    U,
    V,
}

/// Scalar YCbCr/YCgCo -> RGBA8 conversion for >8-bit pictures.
///
/// Done by hand because yuvutils-rs 0.8.3 gets >8-bit conversions wrong: its
/// SIMD kernels corrupt the alpha channel and its BT.2020 inverse mis-scales
/// the red channel.
///
/// Chroma is nearest-neighbor upsampled; BT.2020 constant luminance is treated
/// as non-constant luminance.
fn high_depth_ycbcr_to_rgba(
    planes: &Planes,
    used: usize,
    full_range: bool,
    transform: &ColorTransform,
    rgba: &mut [u8],
) {
    let width = planes.width;
    let height = planes.height;
    // chroma sits at half resolution horizontally for 4:2:0/4:2:2 and
    // vertically only for 4:2:0
    let chroma_down_x = planes.layout != PixelLayout::I444;
    let chroma_down_y = planes.layout == PixelLayout::I420;

    for row in 0..height {
        let chroma_row = if chroma_down_y { row / 2 } else { row };
        let out = &mut rgba[row * width * 4..][..width * 4];
        for (x, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let chroma_x = if chroma_down_x { x / 2 } else { x };

            let y = expand_luma(planes.sample(Sample::Y, x, row) as u32, used, full_range);

            let (r, g, b) = match transform {
                ColorTransform::Ycbcr { kr, kb, .. } => {
                    let cb = expand_chroma(
                        planes.sample(Sample::U, chroma_x, chroma_row) as u32,
                        used,
                        full_range,
                    );
                    let cr = expand_chroma(
                        planes.sample(Sample::V, chroma_x, chroma_row) as u32,
                        used,
                        full_range,
                    );
                    let kb_g = 2.0 * kb * (1.0 - kb) / (1.0 - kr - kb);
                    let kr_g = 2.0 * kr * (1.0 - kr) / (1.0 - kr - kb);
                    (
                        y + (2.0 - 2.0 * kr) * cr,
                        y - kb_g * cb - kr_g * cr,
                        y + (2.0 - 2.0 * kb) * cb,
                    )
                }
                ColorTransform::Ycgco => {
                    let cg = expand_chroma(
                        planes.sample(Sample::U, chroma_x, chroma_row) as u32,
                        used,
                        full_range,
                    );
                    let co = expand_chroma(
                        planes.sample(Sample::V, chroma_x, chroma_row) as u32,
                        used,
                        full_range,
                    );
                    (y - cg + co, y + cg, y - cg - co)
                }
            };

            px[0] = clamp_u8(r);
            px[1] = clamp_u8(g);
            px[2] = clamp_u8(b);
            px[3] = 255;
        }
    }
}

/// Maps a luma sample to the 8-bit domain.
fn expand_luma(sample: u32, used: usize, full_range: bool) -> f32 {
    if full_range {
        sample as f32 * 255.0 / (((1u32 << used) - 1) as f32)
    } else {
        let floor = 16u32 << (used - 8);
        let span = 219u32 << (used - 8);
        (sample.saturating_sub(floor)) as f32 * 255.0 / span as f32
    }
}

/// Maps a chroma sample to a signed offset in the 8-bit domain.
fn expand_chroma(sample: u32, used: usize, full_range: bool) -> f32 {
    let mid = 1u32 << (used - 1);
    if full_range {
        (sample as i32 - mid as i32) as f32 * 255.0 / mid as f32
    } else {
        let floor = 16u32 << (used - 8);
        let span = 224u32 << (used - 8);
        (sample.saturating_sub(floor) as i32 - (span / 2) as i32) as f32 * 255.0 / span as f32
    }
}

fn clamp_u8(value: f32) -> u8 {
    (value + 0.5).clamp(0.0, 255.0) as u8
}

fn rgba_stride(width: usize) -> u32 {
    (width * 4) as u32
}

/// Number of bits actually used by the samples of a picture.
///
/// dav1d only ever reports 8, 10 or 12, but the value is clamped to 8..=16
/// anyway: below 8 the `used - 8` shifts in `expand_luma`/`expand_chroma`
/// underflow, and above 16 the `1 << used` shifts overflow.
fn used_bits(picture: &Picture) -> Result<usize, ImageErrors> {
    picture
        .bits_per_component()
        .map(|bits| bits.0.clamp(8, 16))
        .ok_or_else(|| decode_error("unknown bit depth"))
}

/// Reads a little-endian sample and scales it down to 8 bits.
///
/// Returns 0 when the slice is too short to hold a full 2-byte sample, which
/// can happen with a corrupted bitstream or mismatched stride. Returning a
/// neutral value is safer than panicking across the dav1d FFI boundary.
fn sample_u8(bytes: &[u8], used: usize) -> u8 {
    let Some(arr) = bytes.get(..2) else {
        return 0;
    };
    let value = u16::from_le_bytes([arr[0], arr[1]]) as u32;
    let max = ((1u32 << used) - 1).max(1);
    ((value * 255 + max / 2) / max) as u8
}

#[cfg(test)]
mod tests;
