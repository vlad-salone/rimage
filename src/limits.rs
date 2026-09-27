//! Runtime-derived image size limits.
//!
//! rimage decodes a whole image into an interleaved pixel buffer, keeps one or
//! more copies alive through resize / quantize, and then hands another copy to
//! the encoder. Peak memory therefore scales with the pixel count, which means
//! the largest acceptable image is a property of the *machine*, not a constant
//! baked into the source.
//!
//! This module derives that ceiling at runtime from three independent sources:
//!
//! 1. [`format_caps`] — dimensions a format itself declares as hard limits.
//!    Only values with an authoritative, citable source are listed here.
//! 2. [`SystemBudget`] — memory available to this process, divided by the
//!    concurrency and the peak number of live pixel buffers.
//! 3. [`LimitSet::for_output`] — free space on the destination volume, which
//!    bounds the size of the file that can be written.
//!
//! The effective limit is the intersection of the three. No fixed pixel
//! threshold is hardcoded; the only hardcoded numbers are the `format_caps`
//! values that the format specifications themselves mandate, plus the
//! documented estimates in [`PipelineCost`].

use std::path::Path;
#[cfg(feature = "limits")]
use std::path::PathBuf;

use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace};

/// Upper bound on a single dimension, and on the total pixel count, declared by
/// a format rather than derived from system resources.
///
/// A value of [`u64::MAX`] means the format publishes no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatCaps {
    /// Maximum width or height, in pixels.
    pub max_side: u64,
    /// Maximum `width * height`, in pixels.
    pub max_pixels: u64,
    /// Human-readable origin of these values, for error messages.
    pub source: &'static str,
}

impl FormatCaps {
    /// No published limit.
    pub const UNBOUNDED: Self = Self {
        max_side: u64::MAX,
        max_pixels: u64::MAX,
        source: "no published limit",
    };
}

/// Dimensions a format declares as hard limits.
///
/// Only values backed by an authoritative statement are listed. Everything
/// else falls back to [`FormatCaps::UNBOUNDED`] and is bounded purely by the
/// runtime memory budget, so a library that merely *happens* to reject a size
/// today does not get frozen into the source.
///
/// # Sources
///
/// - WebP: `WEBP_MAX_DIMENSION` in libwebp's `src/webp/encode.h`, documented as
///   the inclusive maximum width/height. This is the codec's own published
///   constant, so it is safe to encode here.
/// - AVIF: the AV1 bitstream limit for a coded image, stated in the AVIF
///   specification (Profiles Overview) as 65536x65536 at `seq_level_idx=31`.
///   This bounds what a decoder must accept; it is *not* a promise that the
///   encoder will succeed at that size, so the memory budget still applies.
/// - JPEG and PNG: the zune decoders reject anything above the `max_width` /
///   `max_height` in their `DecoderOptions`, which defaults to 16384 and is
///   documented as respected by *all* decoders. This is deliberately not the
///   larger figure the underlying codecs advertise (libjpeg's
///   `JPEG_MAX_DIMENSION` is 65500): a limit that the decoder refuses before
///   the codec is reached is the one a pre-check has to agree with, or the
///   check passes a file that is then rejected further down with a worse
///   message. Both decoders read the header before allocating, so 16384 is
///   the size at which the pipeline stops, measured rather than guessed.
/// - Anything absent from this table is unbounded here and left to the memory
///   budget plus the codec's own error reporting.
pub const fn format_caps(format: ImageFormatId) -> FormatCaps {
    match format {
        ImageFormatId::WebP => FormatCaps {
            max_side: 16383,
            max_pixels: u64::MAX,
            source: "libwebp WEBP_MAX_DIMENSION",
        },
        ImageFormatId::Avif => FormatCaps {
            max_side: 65536,
            max_pixels: u64::MAX,
            source: "AVIF spec: AV1 coded image limit at seq_level_idx=31",
        },
        // The zune decoder's own ceiling, which it applies before the codec
        // ever runs; see the module comment above.
        ImageFormatId::Jpeg => FormatCaps {
            max_side: DECODER_SIDE_LIMIT,
            max_pixels: u64::MAX,
            source: "zune-jpeg DecoderOptions::max_width default",
        },
        ImageFormatId::Png => FormatCaps {
            max_side: DECODER_SIDE_LIMIT,
            max_pixels: u64::MAX,
            source: "zune-png DecoderOptions::max_width default",
        },
        _ => FormatCaps::UNBOUNDED,
    }
}

/// Side limit the zune decoders apply by default.
///
/// `zune_core::options::DecoderOptions` documents `max_width` and `max_height`
/// as 16384 and "respected by all decoders". Both `zune-jpeg` and `zune-png`
/// check it against the header before allocating, so it is a real ceiling
/// rather than a suggestion.
const DECODER_SIDE_LIMIT: u64 = 16384;

/// Format identity used for limit lookup.
///
/// Kept as a small standalone enum so this module does not depend on the
/// encoder dispatch types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormatId {
    /// JPEG, via `mozjpeg`.
    Jpeg,
    /// PNG, via `oxipng` / `imagequant`.
    Png,
    /// WebP, via `libwebp`.
    WebP,
    /// AVIF, via `ravif`.
    Avif,
    /// TIFF, via the `tiff` crate.
    Tiff,
    /// SVG, rasterised via `resvg` / `tiny-skia`.
    Svg,
    /// A format with no specific handling; treated as unconstrained.
    Other,
}

impl ImageFormatId {
    /// Canonical lowercase name, used in messages.
    pub const fn name(self) -> &'static str {
        match self {
            ImageFormatId::Jpeg => "jpeg",
            ImageFormatId::Png => "png",
            ImageFormatId::WebP => "webp",
            ImageFormatId::Avif => "avif",
            ImageFormatId::Tiff => "tiff",
            ImageFormatId::Svg => "svg",
            ImageFormatId::Other => "image",
        }
    }

    /// Map a file extension to a format identity.
    pub fn from_extension(ext: &str) -> Self {
        if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
            ImageFormatId::Jpeg
        } else if ext.eq_ignore_ascii_case("png") {
            ImageFormatId::Png
        } else if ext.eq_ignore_ascii_case("webp") {
            ImageFormatId::WebP
        } else if ext.eq_ignore_ascii_case("avif") {
            ImageFormatId::Avif
        } else if ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff") {
            ImageFormatId::Tiff
        } else if ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz") {
            ImageFormatId::Svg
        } else {
            ImageFormatId::Other
        }
    }

    /// Map a CLI encoder name to a format identity.
    ///
    /// These are the subcommand names, which do not always match the file
    /// extension: `mozjpeg` writes a `.jpg`, and `oxipng` writes a `.png`.
    /// Output limits must be looked up by what will actually be written, so
    /// the encoder name is the right key here rather than the path suffix.
    pub fn from_encoder_name(name: &str) -> Self {
        match name {
            "mozjpeg" | "jpeg" => ImageFormatId::Jpeg,
            "oxipng" | "png" => ImageFormatId::Png,
            "webp" => ImageFormatId::WebP,
            "avif" => ImageFormatId::Avif,
            "tiff" => ImageFormatId::Tiff,
            _ => ImageFormatId::Other,
        }
    }

    /// The widest pixel layout a decoder may hand back for this format.
    ///
    /// A size screen that runs *before* decoding cannot ask the image what it
    /// holds, so it has to assume, and the only safe direction is to assume
    /// the widest layout the format allows. Under-estimating admits an image
    /// that then exhausts memory inside a codec; over-estimating merely turns
    /// a large-but-handled image into a message telling the user to resize.
    ///
    /// These are ceilings, not typical values: an ordinary 8-bit RGB JPEG is
    /// 3 bytes per pixel, not the 4 this returns. That gap is why the
    /// pipeline re-checks the real layout once the image is decoded, where
    /// `Image::depth` and `Image::colorspace` are known exactly.
    pub const fn max_pixel_layout(self) -> (BitDepth, ColorSpace) {
        match self {
            // 8-bit only, but a four-component (CMYK/YCCK) source is legal.
            ImageFormatId::Jpeg => (BitDepth::Eight, ColorSpace::RGBA),
            // 16-bit samples and an alpha channel are both legal.
            ImageFormatId::Png => (BitDepth::Sixteen, ColorSpace::RGBA),
            ImageFormatId::WebP => (BitDepth::Eight, ColorSpace::RGBA),
            // High-bit-depth sources are converted to 8-bit on decode and
            // alpha is always carried, so the ceiling is 8-bit RGBA.
            ImageFormatId::Avif => (BitDepth::Eight, ColorSpace::RGBA),
            ImageFormatId::Tiff => (BitDepth::Sixteen, ColorSpace::RGBA),
            // Rasterised at 8 bits with alpha.
            ImageFormatId::Svg => (BitDepth::Eight, ColorSpace::RGBA),
            // Nothing is known about the file, so assume the widest layout
            // anything this program decodes can produce.
            ImageFormatId::Other => (BitDepth::Sixteen, ColorSpace::RGBA),
        }
    }
}

/// Fraction of the reported free memory this process is willing to spend.
///
/// `available_memory()` reports memory the kernel could hand out *right now*.
/// It is not a promise: other processes compete for it, the page cache is
/// reclaimable but not instant, and rimage's own peak is a short spike that
/// arrives after several allocations have already succeeded. Spending all of it
/// invites the OOM killer (or, on Windows, a swap storm) instead of a clean
/// error, so claim at most half by default.
const MEMORY_SAFETY_NUM: u64 = 1;
const MEMORY_SAFETY_DEN: u64 = 2;

/// Largest contiguous allocation assumed to be obtainable on a 32-bit process.
///
/// A `Vec<u8>` needs one contiguous virtual range. On 32-bit targets the
/// address space is 2 GiB (4 GiB with `LARGEADDRESSAWARE`), and fragmentation
/// routinely prevents a single large block even when the free total looks
/// sufficient — that is the difference between a failed allocation and a real
/// OOM. Cap below the theoretical maximum to leave room for the rest of the
/// process image.
#[cfg(target_pointer_width = "32")]
const ADDRESS_SPACE_CAP: u64 = 768 * 1024 * 1024;

/// On 64-bit targets the address space is not the binding constraint.
#[cfg(not(target_pointer_width = "32"))]
const ADDRESS_SPACE_CAP: u64 = u64::MAX;

/// Fallback per-image budget when the machine cannot be probed.
///
/// Probing fails on unsupported platforms, inside restricted sandboxes, or when
/// a network filesystem stalls. Falling back to a fixed, conservative budget
/// keeps the tool usable; refusing to run would be worse.
const FALLBACK_PER_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

/// Bytes reserved on the destination volume for filesystem metadata and
/// unrelated activity between the free-space check and the write.
const DISK_RESERVE_BYTES: u64 = 100 * 1024 * 1024;

/// Peak number of live full-image buffers, as a multiplier on
/// `pixels * bytes_per_pixel`.
///
/// These are *estimates of the current pipeline*, not format limits. They are
/// deliberately kept as named constants with a documented basis so they can be
/// recalibrated from measurement rather than guessed at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineCost {
    /// Buffers alive at peak while decoding.
    pub decode: u64,
    /// Extra buffers introduced by `--resize`.
    pub resize: u64,
    /// Extra buffers introduced by `--quantization`.
    pub quantize: u64,
    /// Peak buffers held by the encoder itself.
    pub encode: u64,
}

impl PipelineCost {
    /// Cost of decoding plus encoding with no preprocessing.
    ///
    /// `decode` is 3: for the SVG path this is verified directly from the code
    /// (premultiplied pixmap, straight-alpha interleaved copy, and the
    /// deinterleaved channel buffers). Other decoders are assumed to be no
    /// worse, so 3 is a floor rather than a measurement.
    ///
    /// `encode` differs per codec. Entero-coding and blocking encoders
    /// (`oxipng`, `ravif`) build substantially more scratch state than
    /// DCT-based ones (`mozjpeg`).
    pub const fn new(decode: u64, resize: u64, quantize: u64, encode: u64) -> Self {
        Self {
            decode,
            resize,
            quantize,
            encode,
        }
    }

    /// Total live buffers at peak.
    pub const fn total(&self) -> u64 {
        self.decode + self.resize + self.quantize + self.encode
    }

    /// Cost for an encoder chosen by format identity.
    ///
    /// The encode multipliers are estimates with LOW confidence; they exist to
    /// keep the derived limit in the right order of magnitude, not to promise a
    /// precise figure. Lower `--speed`/higher effort settings raise them.
    pub const fn for_encoder(format: ImageFormatId) -> Self {
        let encode = match format {
            // DCT-based, one coefficient buffer per component.
            ImageFormatId::Jpeg => 3,
            // Deflate/filter search keeps several filtered scanline sets.
            ImageFormatId::Png => 12,
            // AV1 partitions and mode decision keep large scratch planes.
            ImageFormatId::Avif => 16,
            // Prediction/filter loops over a full plane.
            ImageFormatId::WebP => 6,
            _ => 8,
        };
        Self::new(3, 0, 0, encode)
    }

    /// Cost of a whole conversion: decoding `input`, then encoding to `output`.
    ///
    /// The two halves belong to different formats, and taking both from one of
    /// them is what screened a JPEG-to-AVIF conversion against JPEG's
    /// 6-buffer estimate while the AVIF encoder went on to hold 19: the encode
    /// buffers dominate, and it is the *output* format that decides how many
    /// there are. The arguments are therefore not interchangeable — swapping
    /// them changes the answer.
    pub const fn for_conversion(input: ImageFormatId, output: ImageFormatId) -> Self {
        let decoded = Self::for_encoder(input);
        let encoded = Self::for_encoder(output);

        Self::new(
            decoded.decode,
            decoded.resize,
            decoded.quantize,
            encoded.encode,
        )
    }
}

/// Runtime-probed view of the resources available to this process.
///
/// Probed once and cached, because `sysinfo` refreshes touch the whole process
/// table and free memory is only needed to size a budget, not to track changes.
#[derive(Debug, Clone, Copy)]
pub struct SystemBudget {
    /// Free memory in bytes, or 0 when the platform could not be probed.
    pub available_memory: u64,
    /// Largest byte allocation assumed obtainable in one contiguous range.
    pub address_cap: u64,
    /// Number of images processed simultaneously.
    pub concurrency: usize,
}

impl SystemBudget {
    /// Probe the current system.
    ///
    /// `concurrency` is the number of images the caller processes at once; it
    /// divides the budget because every concurrent image holds its own buffers.
    pub fn probe(concurrency: usize) -> Self {
        Self {
            available_memory: probe_available_memory(),
            address_cap: ADDRESS_SPACE_CAP,
            concurrency: concurrency.max(1),
        }
    }

    /// Budget for one image, in bytes.
    ///
    /// Falls back to [`FALLBACK_PER_IMAGE_BYTES`] when the machine could not be
    /// probed. Never returns 0: callers divide by this value.
    pub fn per_image_bytes(&self) -> u64 {
        if self.available_memory == 0 {
            return FALLBACK_PER_IMAGE_BYTES;
        }

        let usable = (self.available_memory / MEMORY_SAFETY_DEN) * MEMORY_SAFETY_NUM;

        // `probe()` normalises this, but the struct is `pub` and this method is
        // the divisor for every pixel budget, so guard against a hand-built
        // zero rather than risk a division panic.
        let concurrency = self.concurrency.max(1) as u64;

        (usable.min(self.address_cap) / concurrency).max(1)
    }

    /// Whether the reported figure came from a real probe.
    pub fn is_probed(&self) -> bool {
        self.available_memory != 0
    }
}

/// Query free memory, preferring a container-aware source.
///
/// On Linux, `/proc/meminfo` reports the *host's* memory and ignores any cgroup
/// limit, so a container sees far more than it may actually use. cgroup limits
/// are therefore consulted first and take the smaller of the two.
#[cfg(feature = "limits")]
fn probe_available_memory() -> u64 {
    use sysinfo::{MemoryRefreshKind, System};

    let mut system = System::new();
    system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

    // `cgroup_limits()` is Linux-only and returns `None` elsewhere.
    match system.cgroup_limits() {
        Some(limits) if limits.total_memory > 0 => {
            limits.free_memory.min(system.available_memory())
        }
        _ => system.available_memory(),
    }
}

#[cfg(not(feature = "limits"))]
fn probe_available_memory() -> u64 {
    0
}

/// Free bytes on the volume holding `path`, if it can be determined.
///
/// Resolved by longest mount-point prefix so the answer reflects the volume the
/// output actually lands on, not the working directory. `path` usually does not
/// exist yet — it is the output being *planned* — so the lookup walks up to the
/// nearest ancestor that does and canonicalizes that; a plain
/// `path.canonicalize()` would fail on exactly the new-output-file case this
/// function exists for. Returns `None` when no ancestor exists, the volume is
/// not enumerated, or the platform cannot report it; callers must treat that
/// as "unknown", not "zero".
#[cfg(feature = "limits")]
pub fn free_space_at(path: &Path) -> Option<u64> {
    use sysinfo::Disks;

    let target = canonicalize_existing_ancestor(path)?;

    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|disk| target.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
}

/// Canonicalize the nearest ancestor of `path` that exists.
///
/// The verbatim (`\\?\`) prefix `canonicalize` produces on Windows is stripped
/// because sysinfo reports mount points in the ordinary `C:\` form; comparing
/// the verbatim form against them would never match and silently disable the
/// free-space check on all of Windows.
#[cfg(feature = "limits")]
fn canonicalize_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if let Ok(canonical) = current.canonicalize() {
            return Some(strip_verbatim_prefix(&canonical));
        }
        candidate = current.parent();
    }
    None
}

/// Strip the Windows verbatim (`\\?\`) prefix from a canonicalized path.
///
/// `\\?\UNC\server\share` is restored to the regular UNC form. Paths without
/// the prefix — and every path on non-Windows platforms — are returned
/// unchanged.
#[cfg(feature = "limits")]
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(s) = path.to_str() {
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{rest}"));
            }
            if let Some(rest) = s.strip_prefix(r"\\?\") {
                return PathBuf::from(rest);
            }
        }
    }
    path.to_path_buf()
}

/// Free bytes on the volume holding `path`, if it can be determined.
///
/// Without the `limits` feature there is nothing to measure, because the
/// platform query lives behind it. This always reports "unknown", which is
/// the same answer the enabled version gives for a volume it cannot
/// enumerate — callers must not read it as "no space left".
#[cfg(not(feature = "limits"))]
pub fn free_space_at(_path: &Path) -> Option<u64> {
    None
}

/// A resolved set of limits for one decode or encode operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitSet {
    /// Maximum width in pixels.
    pub max_width: u64,
    /// Maximum height in pixels.
    pub max_height: u64,
    /// Maximum `width * height` in pixels.
    pub max_pixels: u64,
    /// Maximum input or output size in bytes, when known.
    pub max_bytes: u64,
    /// Which constraint produced the tightest `max_pixels`, for messages.
    pub binding: Binding,
    /// Which constraint produced `max_bytes`.
    ///
    /// Tracked separately from `binding` because the two ceilings are bounded
    /// by different sources: the pixel ceiling is never set by free space, and
    /// the byte ceiling is only meaningful as a disk statement when free space
    /// is what tightened it. Collapsing them into one field made a memory
    /// budget get reported as a disk limit.
    pub bytes_binding: Binding,
}

/// Which of the three sources ended up being the tightest constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// A dimension limit declared by the format.
    Format,
    /// The memory budget derived from system resources.
    Memory,
    /// Free space on the destination volume.
    Disk,
    /// Nothing constrained the operation.
    None,
}

impl Binding {
    /// Human-readable explanation, used in error messages.
    pub const fn describe(self) -> &'static str {
        match self {
            Binding::Format => "the format's own dimension limit",
            Binding::Memory => "the available memory budget",
            Binding::Disk => "free space on the destination volume",
            Binding::None => "no limit",
        }
    }
}

impl LimitSet {
    /// Limits for reading an image of the given format.
    ///
    /// The pixel ceiling is whichever of the format's published limit and the
    /// memory budget is smaller. A single side may still pass the side check
    /// while the product fails, which is why `max_pixels` is tracked
    /// separately: callers that only know one dimension must not assume the
    /// other is unconstrained.
    pub fn for_input(
        format: ImageFormatId,
        depth: BitDepth,
        colorspace: ColorSpace,
        budget: &SystemBudget,
        cost: PipelineCost,
    ) -> Self {
        let caps = format_caps(format);
        let bytes_per_pixel = bytes_per_pixel(depth, colorspace);

        let budget_bytes = budget.per_image_bytes();
        let budget_pixels = budget_bytes / (bytes_per_pixel * cost.total().max(1));

        // A format may publish a per-side limit without a pixel limit (and
        // libjpeg is lossy, so memory is no guide either). Since every sample
        // still has to be held somewhere, a square of `max_side` is the
        // smallest rectangle that reaches the side ceiling; anything with a
        // larger area is necessarily wider or taller than `max_side`. Deriving
        // this keeps `max_pixels` meaningful instead of leaving it at `u64::MAX`.
        let format_pixels = caps
            .max_side
            .saturating_mul(caps.max_side)
            .min(caps.max_pixels);

        let (max_pixels, binding) = if format_pixels <= budget_pixels {
            (format_pixels, Binding::Format)
        } else {
            (budget_pixels, Binding::Memory)
        };

        Self {
            max_width: caps.max_side,
            max_height: caps.max_side,
            max_pixels,
            max_bytes: budget_bytes.saturating_mul(cost.total().max(1)),
            binding,
            bytes_binding: Binding::Memory,
        }
    }

    /// Limits for writing an image of the given format to `output`.
    ///
    /// Adds the destination volume's free space on top of the input limits, so
    /// a huge image is rejected before it is encoded rather than after a
    /// partial write.
    pub fn for_output(
        format: ImageFormatId,
        depth: BitDepth,
        colorspace: ColorSpace,
        budget: &SystemBudget,
        cost: PipelineCost,
        output: &Path,
    ) -> Self {
        let mut limits = Self::for_input(format, depth, colorspace, budget, cost);

        if let Some(free) = free_space_at(output) {
            let usable = free.saturating_sub(DISK_RESERVE_BYTES);
            // A lossless encoder can expand its input, and a lossy one still
            // needs headroom for the container and metadata. Require the volume
            // to hold twice the decoded budget when that is achievable.
            let allowed = usable / 2;
            if allowed < limits.max_bytes {
                limits.max_bytes = allowed;
                limits.bytes_binding = Binding::Disk;
            }
        }

        limits
    }

    /// Check a concrete `width x height` pair.
    ///
    /// Returns the constraint that was violated, so the caller can name it in
    /// the error message instead of reporting a bare boolean.
    pub fn check(&self, width: u64, height: u64) -> Result<(), LimitViolation> {
        // Saturating arithmetic throughout: a `width * height` that overflows
        // `u64` is not a value to panic on, it is an input to reject.
        let pixels = width.saturating_mul(height);
        if pixels > self.max_pixels {
            return Err(LimitViolation {
                kind: ViolationKind::Pixels,
                actual: pixels,
                allowed: self.max_pixels,
                binding: self.binding,
            });
        }

        if width > self.max_width {
            return Err(LimitViolation {
                kind: ViolationKind::Width,
                actual: width,
                allowed: self.max_width,
                binding: Binding::Format,
            });
        }

        if height > self.max_height {
            return Err(LimitViolation {
                kind: ViolationKind::Height,
                actual: height,
                allowed: self.max_height,
                binding: Binding::Format,
            });
        }

        Ok(())
    }

    /// Check an estimated byte footprint against the byte ceiling.
    ///
    /// Distinct from [`LimitSet::check`] because the two ceilings answer
    /// different questions: `check` asks whether the pixels fit in *memory*,
    /// this asks whether the result fits on *disk*. A small-pixel image in a
    /// pathological format can still exhaust a volume, and a huge one may fit
    /// on disk while never surviving the decode.
    pub fn check_bytes(&self, bytes: u64) -> Result<(), LimitViolation> {
        if bytes > self.max_bytes {
            return Err(LimitViolation {
                kind: ViolationKind::Bytes,
                actual: bytes,
                allowed: self.max_bytes,
                binding: self.bytes_binding,
            });
        }

        Ok(())
    }

    /// Largest square side that satisfies the pixel ceiling.
    ///
    /// Used to suggest a concrete `--resize` value in error messages.
    pub fn suggested_side(&self) -> u64 {
        let by_pixels = self.max_pixels.isqrt();
        by_pixels.min(self.max_width).min(self.max_height).max(1)
    }
}

/// Which measurement exceeded its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// Width alone.
    Width,
    /// Height alone.
    Height,
    /// `width * height`.
    Pixels,
    /// An estimated byte footprint, bounded by free space on the volume.
    Bytes,
}

/// A concrete limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitViolation {
    /// What was measured.
    pub kind: ViolationKind,
    /// The offending value.
    pub actual: u64,
    /// The ceiling it exceeded.
    pub allowed: u64,
    /// Which source produced the ceiling.
    pub binding: Binding,
}

/// Bytes needed per pixel for a bit depth and colour space.
pub fn bytes_per_pixel(depth: BitDepth, colorspace: ColorSpace) -> u64 {
    let components = colorspace.num_components() as u64;
    let bytes_per_component = match depth {
        BitDepth::Eight => 1,
        BitDepth::Sixteen => 2,
        // Float images are stored as 32-bit samples.
        BitDepth::Float32 => 4,
        // Both `Unknown` and any variant added upstream describe samples we
        // cannot size precisely; one byte per component is the floor, so the
        // resulting budget is never an over-estimate.
        _ => 1,
    };

    (components * bytes_per_component).max(1)
}

#[cfg(test)]
mod tests;
