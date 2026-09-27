//! SVG decoder rendering vector images into raster pixels through `resvg`.

use std::io::Read;
use std::path::PathBuf;

use resvg::tiny_skia;
use resvg::usvg;
use zune_core::colorspace::ColorSpace;
use zune_image::errors::ImageErrors;
use zune_image::image::Image;
use zune_image::traits::DecoderTrait;

use super::fonts;

/// Total bytes that may be live simultaneously while an SVG is decoded.
///
/// Decoding keeps three pixel-sized buffers alive at peak:
/// the premultiplied `tiny_skia::Pixmap`, the straight-alpha interleaved
/// copy built by [`SvgDecoder::decode`], and the deinterleaved channel
/// buffers inside the resulting [`Image`]. The pixel limit is derived from
/// this budget so a valid SVG cannot force an out-of-memory abort.
const MAX_SVG_DECODE_BYTES: u64 = 512 * 1024 * 1024;

const SVG_BYTES_PER_PIXEL: u64 = 4;

const SVG_SIMULTANEOUS_PIXEL_BUFFERS: u64 = 3;

/// Maximum number of pixels an SVG render target may cover by default.
///
/// Used when a caller does not supply its own budget through
/// [`SvgOptions::pixel_budget`]. The runtime-derived limit from
/// `rimage::limits` is preferred; this constant is the fallback so the decoder
/// is still safe when used on its own.
pub const MAX_TARGET_PIXELS: u64 =
    MAX_SVG_DECODE_BYTES / (SVG_BYTES_PER_PIXEL * SVG_SIMULTANEOUS_PIXEL_BUFFERS);

/// Options controlling how an SVG image is rendered into pixels.
#[derive(Clone, Debug, Default)]
pub struct SvgOptions {
    /// Directory used to resolve relative paths inside the SVG, such as the
    /// `href` of an `<image>` element.
    ///
    /// Should be set to the directory containing the SVG file.
    pub resources_dir: Option<PathBuf>,
    /// Explicit render target in pixels. When `None`, the SVG is rendered
    /// at its intrinsic size.
    ///
    /// The resolved render target may not exceed the pixel budget.
    pub target_size: Option<(u32, u32)>,
    /// Maximum pixels the render target may cover.
    ///
    /// `None` falls back to [`MAX_TARGET_PIXELS`]. Callers that can probe the
    /// machine should pass a budget derived from
    /// [`crate::limits::SystemBudget`] instead, so the ceiling tracks the
    /// memory actually available rather than a constant.
    pub pixel_budget: Option<u64>,
}

/// A decoder that renders SVG images into raster pixels using `resvg`.
///
/// Scaling happens while rendering, so any target size keeps the vector
/// quality of the source instead of resampling a rasterized image.
pub struct SvgDecoder {
    tree: usvg::Tree,
    /// The raw, unrounded intrinsic size.
    ///
    /// Kept as f32 because the decode-time scale factor must use the exact
    /// value: rendering a 99.6px-wide vector into a 100px target calls for a
    /// scale of 100/99.6, not 100/100. Callers that need integer pixels (the
    /// resize callback) get their own rounded, `.max(1)`-clamped copy at the
    /// call site.
    intrinsic: (f32, f32),
    target: (usize, usize),
}

/// Upper bound on an SVG/SVGZ document read into memory before parsing.
///
/// `Tree::from_data` reads the whole buffer, and a hostile multi-gigabyte
/// "svg" would exhaust memory before the pixel budget check could run. 256
/// MiB is far beyond any real SVG; the render target is bounded separately.
const MAX_SVG_BYTES: u64 = 256 * 1024 * 1024;

/// Parses an SVG document into a `resvg` tree.
///
/// Reads the whole source, configures `resvg` with the SVG's resource
/// directory and the process-wide system font database, and parses the
/// document. `Tree::from_data` detects and decompresses gzip (SVGZ)
/// automatically.
fn parse_tree<R: Read>(
    source: R,
    resources_dir: Option<PathBuf>,
) -> Result<usvg::Tree, ImageErrors> {
    let mut data = Vec::new();
    source
        .take(MAX_SVG_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| ImageErrors::ImageDecodeErrors(format!("Unable to read SVG data - {e}")))?;
    if data.len() as u64 > MAX_SVG_BYTES {
        return Err(ImageErrors::ImageDecodeErrors(format!(
            "SVG input exceeds the {} MiB read limit",
            MAX_SVG_BYTES / 1024 / 1024
        )));
    }

    let mut usvg_options = usvg::Options {
        resources_dir,
        font_resolver: fonts::font_resolver(),
        ..usvg::Options::default()
    };
    usvg_options.fontdb = fonts::system_fontdb();

    usvg::Tree::from_data(&data, &usvg_options)
        .map_err(|e| ImageErrors::ImageDecodeErrors(format!("Unable to parse SVG - {e}")))
}

impl SvgDecoder {
    /// Create a new SVG decoder with default render options.
    pub fn try_new<R: Read>(source: R) -> Result<Self, ImageErrors> {
        Self::try_new_with_options(source, SvgOptions::default())
    }

    /// Parses the SVG once and resolves the render target through a caller
    /// supplied callback.
    ///
    /// The callback receives the intrinsic SVG size as integer pixels and returns
    /// the desired render target, or `Ok(None)` to render at the intrinsic size.
    /// This avoids parsing the input twice when callers need to compute a resize
    /// target from the intrinsic dimensions.
    pub fn try_new_with_resize<R, F>(
        source: R,
        resources_dir: Option<PathBuf>,
        target_for_size: F,
    ) -> Result<Self, ImageErrors>
    where
        R: Read,
        F: FnOnce((usize, usize)) -> Result<Option<(u32, u32)>, ImageErrors>,
    {
        Self::try_new_with_resize_and_budget(source, resources_dir, None, target_for_size)
    }

    /// Same as [`SvgDecoder::try_new_with_resize`], but with an explicit pixel
    /// budget instead of the module's fallback constant.
    ///
    /// Callers that can probe the machine pass a limit derived from
    /// [`crate::limits::SystemBudget`] here, so an SVG render target is bounded
    /// by the same memory model every other format uses.
    pub fn try_new_with_resize_and_budget<R, F>(
        source: R,
        resources_dir: Option<PathBuf>,
        pixel_budget: Option<u64>,
        target_for_size: F,
    ) -> Result<Self, ImageErrors>
    where
        R: Read,
        F: FnOnce((usize, usize)) -> Result<Option<(u32, u32)>, ImageErrors>,
    {
        let tree = parse_tree(source, resources_dir.clone())?;
        let size = tree.size();
        let intrinsic = (
            (size.width().round() as usize).max(1),
            (size.height().round() as usize).max(1),
        );
        let target_size = target_for_size(intrinsic)?;
        let target = resolve_target_size(
            &SvgOptions {
                resources_dir,
                target_size,
                pixel_budget,
            },
            size,
        )?;

        Ok(Self {
            tree,
            intrinsic: (size.width(), size.height()),
            target,
        })
    }

    /// Returns the intrinsic SVG size in pixels without rendering the image.
    ///
    /// Unlike [`SvgDecoder::try_new_with_options`], this does not validate
    /// [`MAX_TARGET_PIXELS`]; callers can use it to compute a resize target
    /// before seeking back and constructing the real decoder.
    pub fn probe_size<R: Read>(
        source: R,
        resources_dir: Option<PathBuf>,
    ) -> Result<(f32, f32), ImageErrors> {
        let tree = parse_tree(source, resources_dir)?;
        let size = tree.size();
        Ok((size.width(), size.height()))
    }

    /// Create a new SVG decoder with custom render options.
    pub fn try_new_with_options<R: Read>(
        source: R,
        options: SvgOptions,
    ) -> Result<Self, ImageErrors> {
        let tree = parse_tree(source, options.resources_dir.clone())?;

        let size = tree.size();
        let target = resolve_target_size(&options, size)?;

        Ok(Self {
            tree,
            intrinsic: (size.width(), size.height()),
            target,
        })
    }
}

/// Resolves the pixel size the SVG should be rendered at from the requested
/// options and the intrinsic SVG size.
fn resolve_target_size(
    options: &SvgOptions,
    size: usvg::Size,
) -> Result<(usize, usize), ImageErrors> {
    let target = match options.target_size {
        Some((width, height)) => {
            if width == 0 || height == 0 {
                return Err(ImageErrors::ImageDecodeErrors(format!(
                    "Invalid SVG target size {width}x{height}"
                )));
            }
            (width, height)
        }
        None => {
            let intrinsic = size.to_int_size();
            (intrinsic.width(), intrinsic.height())
        }
    };

    // Clamp both dimensions to at least 1x1.
    let width = target.0.max(1) as usize;
    let height = target.1.max(1) as usize;

    // The area is computed in u64 because width and height can each approach
    // u32::MAX, whose product overflows usize.
    let area = (width as u64) * (height as u64);
    let budget = options.pixel_budget.unwrap_or(MAX_TARGET_PIXELS);
    if area > budget {
        return Err(size_limit_error(width, height, area, budget));
    }

    Ok((width, height))
}

/// Marker prefix identifying an SVG render-target rejection.
///
/// `zune_image::errors::ImageErrors` has no variant for "too large", and it is
/// an external type this crate cannot extend. A string is the only channel
/// available, so the one this decoder controls carries a stable marker and the
/// numbers, and [`crate::error::classify_input`] turns it back into a
/// structured failure. Prose alone would force the classifier to pattern-match
/// a human-readable message, which breaks the moment the wording changes.
pub const SIZE_LIMIT_MARKER: &str = "rimage-svg-size-limit:";

/// Build the rejection for a render target that exceeds the pixel budget.
fn size_limit_error(width: usize, height: usize, area: u64, budget: u64) -> ImageErrors {
    ImageErrors::ImageDecodeErrors(format!(
        "{SIZE_LIMIT_MARKER}{width}x{height}:{area}:{budget}: SVG target size {width}x{height} \
         ({area} pixels) exceeds the limit of {budget} pixels, reduce the --resize target or \
         the intrinsic size",
    ))
}

/// Parse a [`SIZE_LIMIT_MARKER`] message back into its numbers.
///
/// Returns `(width, height, actual_pixels, allowed_pixels)`.
pub fn parse_size_limit(message: &str) -> Option<(u64, u64, u64, u64)> {
    let rest = message.strip_prefix(SIZE_LIMIT_MARKER)?;
    // Fields are `WxH:actual:allowed`, terminated by the prose that follows.
    let rest = rest.split_whitespace().next()?;

    let mut fields = rest.split(':');
    let dimensions = fields.next()?;
    let actual = fields.next()?.parse().ok()?;
    let allowed = fields.next()?.parse().ok()?;

    let (width, height) = dimensions.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?, actual, allowed))
}

impl DecoderTrait for SvgDecoder {
    fn decode(&mut self) -> Result<Image, ImageErrors> {
        let (width, height) = self.target;

        let mut pixmap = tiny_skia::Pixmap::new(width as u32, height as u32).ok_or_else(|| {
            ImageErrors::ImageDecodeErrors(format!(
                "Unable to allocate a {width}x{height} pixmap for SVG rendering"
            ))
        })?;

        // Scaling happens here, so the vectors are rasterized directly at the
        // target resolution instead of resampling a smaller image.
        let transform = tiny_skia::Transform::from_scale(
            width as f32 / self.intrinsic.0,
            height as f32 / self.intrinsic.1,
        );

        resvg::render(&self.tree, transform, &mut pixmap.as_mut());

        // tiny-skia stores premultiplied alpha while zune_image expects
        // straight alpha. Use checked_mul to avoid usize overflow on
        // 32-bit targets or extremely large render targets.
        let pixel_count = width
            .checked_mul(height)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| {
                ImageErrors::ImageDecodeErrors(format!(
                    "SVG render target {width}x{height} overflows the output buffer size"
                ))
            })?;
        let mut pixels = Vec::with_capacity(pixel_count);
        for pixel in pixmap.pixels() {
            let color = pixel.demultiply();
            pixels.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }

        Ok(Image::from_u8(&pixels, width, height, ColorSpace::RGBA))
    }

    fn dimensions(&self) -> Option<(usize, usize)> {
        Some(self.target)
    }

    fn out_colorspace(&self) -> ColorSpace {
        ColorSpace::RGBA
    }

    fn name(&self) -> &'static str {
        "svg-decoder"
    }
}

#[cfg(test)]
mod tests;
