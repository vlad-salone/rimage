use zune_core::{
    bit_depth::BitDepth,
    bytestream::{ZByteWriterTrait, ZWriter},
    colorspace::ColorSpace,
};
use zune_image::{
    codecs::ImageFormat,
    errors::{ImageErrors, ImgEncodeErrors},
    image::Image,
    traits::EncoderTrait,
};

/// Alias to [`webp::WebPConfig`]
pub type WebPOptions = webp::WebPConfig;

/// A WebP encoder
pub struct WebPEncoder {
    /// `WebPConfig::new` can fail when libwebp initialisation fails, and
    /// `Default` has no way to report that — so the failure is stored and
    /// surfaced at encode time instead of panicking in a worker thread,
    /// where `panic = "abort"` would take the whole process down.
    options: Option<WebPOptions>,
}

impl Default for WebPEncoder {
    fn default() -> Self {
        Self {
            options: WebPOptions::new().ok(),
        }
    }
}

impl WebPEncoder {
    /// Create a new encoder
    pub fn new() -> WebPEncoder {
        WebPEncoder::default()
    }

    /// Create a new encoder with specified options
    pub fn new_with_options(options: WebPOptions) -> WebPEncoder {
        WebPEncoder {
            options: Some(options),
        }
    }
}

impl EncoderTrait for WebPEncoder {
    fn name(&self) -> &'static str {
        "webp"
    }

    fn encode_inner<T: ZByteWriterTrait>(
        &mut self,
        image: &Image,
        sink: T,
    ) -> Result<usize, ImageErrors> {
        let (width, height) = image.dimensions();

        // WebP's API takes u32 dimensions; `width as u32` would silently
        // truncate a >4 Gpx image and hand libwebp wrong dimensions with the
        // full buffer. Reject it instead.
        let (width_u32, height_u32) = (
            u32::try_from(width).map_err(|_| dimension_overflow(width, height))?,
            u32::try_from(height).map_err(|_| dimension_overflow(width, height))?,
        );

        let mut writer = ZWriter::new(sink);

        if image.is_animated() {
            log::warn!(
                "WebP animation encoding is not supported reliably; only the first frame will be encoded"
            );
        }

        let frames = image.flatten_to_u8();
        let data = frames.first().ok_or({
            ImageErrors::EncodeErrors(ImgEncodeErrors::GenericStatic(
                "Cannot encode an image with no frames",
            ))
        })?;

        let encoder = match image.colorspace() {
            ColorSpace::RGB => webp::Encoder::from_rgb(data, width_u32, height_u32),
            ColorSpace::RGBA => webp::Encoder::from_rgba(data, width_u32, height_u32),
            cs => {
                return Err(ImageErrors::EncodeErrors(
                    ImgEncodeErrors::UnsupportedColorspace(cs, self.supported_colorspaces()),
                ));
            }
        };

        let options = self.options.as_ref().ok_or(ImageErrors::EncodeErrors(
            ImgEncodeErrors::ImageEncodeErrors(
                "libwebp encoder configuration failed to initialize".to_string(),
            ),
        ))?;

        let res = encoder.encode_advanced(options).map_err(|e| {
            ImgEncodeErrors::ImageEncodeErrors(format!("webp encoding failed: {e:?}"))
        })?;

        writer.write(&res).map_err(|e| {
            ImageErrors::EncodeErrors(ImgEncodeErrors::ImageEncodeErrors(format!("{e:?}")))
        })?;

        Ok(writer.bytes_written())
    }

    fn supported_colorspaces(&self) -> &'static [ColorSpace] {
        &[ColorSpace::RGB, ColorSpace::RGBA]
    }

    // TODO: update when new version with custom image format is released.
    fn format(&self) -> ImageFormat {
        ImageFormat::Unknown
    }

    fn supported_bit_depth(&self) -> &'static [BitDepth] {
        &[BitDepth::Eight]
    }

    fn default_depth(&self, _depth: BitDepth) -> BitDepth {
        BitDepth::Eight
    }
}

/// Build the encode error for a dimension that does not fit in WebP's u32.
fn dimension_overflow(width: usize, height: usize) -> ImageErrors {
    ImageErrors::EncodeErrors(ImgEncodeErrors::ImageEncodeErrors(format!(
        "image dimensions {width}x{height} exceed the WebP u32 limit"
    )))
}

#[cfg(test)]
mod tests;
