use std::{io, panic::AssertUnwindSafe};

use mozjpeg::qtable::QTable;
use zune_core::{bit_depth::BitDepth, bytestream::ZByteWriterTrait, colorspace::ColorSpace};
use zune_image::{
    codecs::ImageFormat,
    errors::{ImageErrors, ImgEncodeErrors},
    image::Image,
    traits::EncoderTrait,
};

/// Advanced options for MozJpeg encoding
pub struct MozJpegOptions {
    /// Quality, values 60-80 are recommended. `1..=100`
    pub quality: f32,
    /// Sets progressive mode for image
    pub progressive: bool,
    /// Set to false to make files larger for no reason
    pub optimize_coding: bool,
    /// If `1..=100` (non-zero), it will use MozJPEG's smoothing.
    pub smoothing: u8,
    /// Set color space of JPEG being written, different from input color space
    pub color_space: mozjpeg::ColorSpace,
    /// Specifies whether multiple scans should be considered during trellis quantization.
    pub trellis_multipass: bool,
    /// Sets chroma subsampling, leave as `None` to use auto subsampling
    pub chroma_subsample: Option<u8>,
    /// Instead of quality setting, use a specific quantization table.
    pub luma_qtable: Option<QTable>,
    /// Instead of quality setting, use a specific quantization table for color.
    pub chroma_qtable: Option<QTable>,
}

/// A MozJpeg encoder
#[derive(Default)]
pub struct MozJpegEncoder {
    options: MozJpegOptions,
    pixel_density: Option<mozjpeg::PixelDensity>,
}

struct TempVt<T: ZByteWriterTrait> {
    inner: T,
    bytes_written: usize,
}
impl<T: ZByteWriterTrait> io::Write for TempVt<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let bytes_written = self.inner.write_bytes(buf).map_err(|e| match e {
            zune_core::bytestream::ZByteIoError::StdIoError(e) => e,
            e => io::Error::other(format!("{e:?}")),
        })?;
        self.bytes_written += bytes_written;
        Ok(bytes_written)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.inner.write_all_bytes(buf).map_err(|e| match e {
            zune_core::bytestream::ZByteIoError::StdIoError(e) => e,
            e => io::Error::other(format!("{e:?}")),
        })?;
        self.bytes_written += buf.len();
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush_bytes().map_err(|e| match e {
            zune_core::bytestream::ZByteIoError::StdIoError(e) => e,
            e => io::Error::other(format!("{e:?}")),
        })
    }
}

impl Default for MozJpegOptions {
    fn default() -> Self {
        Self {
            quality: 75.,
            progressive: true,
            optimize_coding: true,
            smoothing: 0,
            color_space: mozjpeg::ColorSpace::JCS_YCbCr,
            trellis_multipass: false,
            chroma_subsample: None,
            luma_qtable: None,
            chroma_qtable: None,
        }
    }
}

impl MozJpegEncoder {
    /// Create a new encoder
    pub fn new() -> MozJpegEncoder {
        MozJpegEncoder::default()
    }

    /// Create a new encoder with specified options
    pub fn new_with_options(options: MozJpegOptions) -> MozJpegEncoder {
        MozJpegEncoder {
            options,
            pixel_density: None,
        }
    }

    /// Set the JFIF pixel density written into the encoded JPEG.
    ///
    /// The value is consumed by the next encode: an encoder used for more
    /// than one image writes the density only into the first file.
    ///
    /// When this is not called, mozjpeg writes its default 1x1 aspect-ratio
    /// JFIF header, which shows up as X/Y Resolution 1 and Resolution Unit
    /// None in tools like ExifTool.
    pub fn set_pixel_density(&mut self, density: mozjpeg::PixelDensity) {
        self.pixel_density = Some(density);
    }
}

impl EncoderTrait for MozJpegEncoder {
    fn name(&self) -> &'static str {
        "mozjpeg-encoder"
    }

    fn encode_inner<T: ZByteWriterTrait>(
        &mut self,
        image: &Image,
        sink: T,
    ) -> Result<usize, ImageErrors> {
        let (width, height) = image.dimensions();

        // The CLI restricts quality to 1..=100, but the library API accepts
        // any f32. mozjpeg's own range assertion would panic — unwindable
        // only because of the catch below, and fatal under the release
        // profile's panic = "abort" — so validate before touching the FFI.
        // Range::contains is false for NaN, which rules that out too.
        if !(1.0..=100.0).contains(&self.options.quality) {
            return Err(ImageErrors::EncodeErrors(ImgEncodeErrors::Generic(
                format!(
                    "mozjpeg quality must be in 1..=100, got {}",
                    self.options.quality
                ),
            )));
        }

        if image.is_animated() {
            log::warn!(
                "MozJpeg does not support animated images, only the first frame will be encoded"
            );
        }
        let frames = image.flatten_to_u8();
        let data = frames.first().ok_or({
            ImageErrors::EncodeErrors(ImgEncodeErrors::GenericStatic(
                "Cannot encode an image with no frames",
            ))
        })?;

        let luma_qtable = self.options.luma_qtable.as_ref();
        let chroma_qtable = self.options.chroma_qtable.as_ref();

        std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<usize, ImageErrors> {
            let format = match image.colorspace() {
                ColorSpace::RGB => mozjpeg::ColorSpace::JCS_RGB,
                ColorSpace::RGBA => mozjpeg::ColorSpace::JCS_EXT_RGBA,
                ColorSpace::YCbCr => mozjpeg::ColorSpace::JCS_YCbCr,
                ColorSpace::Luma => mozjpeg::ColorSpace::JCS_GRAYSCALE,
                ColorSpace::YCCK => mozjpeg::ColorSpace::JCS_YCCK,
                ColorSpace::CMYK => mozjpeg::ColorSpace::JCS_CMYK,
                ColorSpace::BGR => mozjpeg::ColorSpace::JCS_EXT_BGR,
                ColorSpace::BGRA => mozjpeg::ColorSpace::JCS_EXT_BGRA,
                ColorSpace::ARGB => mozjpeg::ColorSpace::JCS_EXT_ARGB,
                _ => mozjpeg::ColorSpace::JCS_UNKNOWN,
            };

            let mut comp = mozjpeg::Compress::new(format);

            comp.set_size(width, height);

            // Custom quantization tables scaled to high quality can produce
            // quantization values approaching the JPEG minimum of 1, which
            // allows DCT coefficient overflow. We apply proportional
            // smoothing to dampen high frequencies before quantization.
            let has_custom_qtables = luma_qtable.is_some() || chroma_qtable.is_some();
            let safe_smoothing = if has_custom_qtables && self.options.quality > 80.0 {
                // q85 → ~4, q90 → ~6, q95 → ~8, q100 → ~10
                let min_smoothing = ((self.options.quality - 75.0) * 0.4).round() as u8;
                let applied = self.options.smoothing.max(min_smoothing);
                if applied != self.options.smoothing {
                    log::debug!(
                        "Smoothing raised from {} to {applied} to prevent DCT overflow at quality {}",
                        self.options.smoothing,
                        self.options.quality,
                    );
                }
                applied
            } else {
                self.options.smoothing
            };

            comp.set_quality(self.options.quality);

            if self.options.progressive {
                comp.set_progressive_mode();
            }

            comp.set_optimize_coding(self.options.optimize_coding);
            comp.set_smoothing_factor(safe_smoothing);
            comp.set_color_space(match format {
                mozjpeg::ColorSpace::JCS_GRAYSCALE => {
                    log::warn!("Input colorspace is GRAYSCALE, using GRAYSCALE as output");

                    mozjpeg::ColorSpace::JCS_GRAYSCALE
                }
                mozjpeg::ColorSpace::JCS_CMYK => {
                    log::warn!("Input colorspace is CMYK, using CMYK as output");

                    mozjpeg::ColorSpace::JCS_CMYK
                }
                mozjpeg::ColorSpace::JCS_YCCK => {
                    log::warn!("Input colorspace is YCCK, using YCCK as output");

                    mozjpeg::ColorSpace::JCS_YCCK
                }

                _ => self.options.color_space,
            });
            comp.set_use_scans_in_trellis(self.options.trellis_multipass);

            if let Some(sb) = self.options.chroma_subsample {
                comp.set_chroma_sampling_pixel_sizes((sb, sb), (sb, sb))
            }

            // Apply custom quantization tables
            // Smoothing (applied above) will handle DCT coefficient normalization
            if let Some(qtable) = luma_qtable {
                comp.set_luma_qtable(qtable)
            }

            if let Some(qtable) = chroma_qtable {
                comp.set_chroma_qtable(qtable)
            }

            if let Some(density) = self.pixel_density.take() {
                comp.set_pixel_density(density);
            }

            let writer = TempVt {
                inner: sink,
                bytes_written: 0,
            };

            let mut comp = comp.start_compress(writer)?;

            #[cfg(feature = "metadata")]
            {
                if let Some(metadata) = &image.metadata().icc_chunk() {
                    comp.write_icc_profile(metadata);
                }
            }

            comp.write_scanlines(data)?;

            Ok(comp.finish()?.bytes_written)
        }))
        .map_err(|err| {
            // Preserve the panic payload whether it was raised as a String
            // or a &'static str; only a foreign payload type falls back to
            // the generic message.
            let message = err
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| err.downcast_ref::<&'static str>().map(|text| (*text).to_string()));
            match message {
                Some(text) => {
                    ImageErrors::EncodeErrors(zune_image::errors::ImgEncodeErrors::Generic(text))
                }
                None => ImageErrors::EncodeErrors(
                    zune_image::errors::ImgEncodeErrors::GenericStatic(
                        "Unknown error occurred during encoding",
                    ),
                ),
            }
        })?
    }

    fn supported_colorspaces(&self) -> &'static [ColorSpace] {
        &[
            ColorSpace::Luma,
            ColorSpace::RGBA,
            ColorSpace::RGB,
            ColorSpace::YCCK,
            ColorSpace::CMYK,
            ColorSpace::BGR,
            ColorSpace::BGRA,
            ColorSpace::ARGB,
            ColorSpace::YCbCr,
        ]
    }

    fn format(&self) -> zune_image::codecs::ImageFormat {
        ImageFormat::JPEG
    }

    fn supported_bit_depth(&self) -> &'static [BitDepth] {
        &[BitDepth::Eight]
    }

    fn default_depth(&self, _depth: BitDepth) -> BitDepth {
        BitDepth::Eight
    }
}

#[cfg(test)]
mod tests;
