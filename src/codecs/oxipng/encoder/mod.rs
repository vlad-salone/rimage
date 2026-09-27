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

/// Alias to [`oxipng::Options`]
pub type OxiPngOptions = oxipng::Options;

/// A OxiPNG encoder
#[derive(Default)]
pub struct OxiPngEncoder {
    options: OxiPngOptions,
}

impl OxiPngEncoder {
    /// Create a new encoder
    pub fn new() -> OxiPngEncoder {
        OxiPngEncoder::default()
    }
    /// Create a new encoder with specified options
    pub fn new_with_options(options: OxiPngOptions) -> OxiPngEncoder {
        OxiPngEncoder { options }
    }
}

impl EncoderTrait for OxiPngEncoder {
    fn name(&self) -> &'static str {
        "oxipng"
    }

    fn encode_inner<T: ZByteWriterTrait>(
        &mut self,
        image: &Image,
        sink: T,
    ) -> Result<usize, ImageErrors> {
        let (width, height) = image.dimensions();

        // PNG stores dimensions as 32-bit; `width as u32` would silently
        // truncate on a hypothetical >4 Gpx image and hand oxipng wrong
        // dimensions with the full buffer. Reject it instead.
        let (width_u32, height_u32) = (
            u32::try_from(width).map_err(|_| dimension_overflow(width, height))?,
            u32::try_from(height).map_err(|_| dimension_overflow(width, height))?,
        );

        if image.is_animated() {
            log::warn!(
                "OxiPNG does not support animated images, only the first frame will be encoded"
            );
        }

        // inlined `to_u8` method because its private
        let colorspace = image.colorspace();
        let data = if image.depth() == BitDepth::Eight {
            image.flatten_frames::<u8>()
        } else if image.depth() == BitDepth::Sixteen {
            image
                .frames_ref()
                .iter()
                .map(|frame| frame.u16_to_big_endian(colorspace))
                .collect()
        } else {
            return Err(ImageErrors::EncodeErrors(ImgEncodeErrors::Generic(
                format!("{:?} is not supported for oxipng encoding", image.depth()),
            )));
        }
        .into_iter()
        .next()
        .ok_or({
            ImageErrors::EncodeErrors(ImgEncodeErrors::GenericStatic(
                "Cannot encode an image with no frames",
            ))
        })?;

        #[allow(unused_mut)]
        let mut img = oxipng::RawImage::new(
            width_u32,
            height_u32,
            match image.colorspace() {
                ColorSpace::Luma => oxipng::ColorType::Grayscale {
                    transparent_shade: None,
                },
                ColorSpace::RGB => oxipng::ColorType::RGB {
                    transparent_color: None,
                },

                ColorSpace::LumaA => oxipng::ColorType::GrayscaleAlpha,
                ColorSpace::RGBA => oxipng::ColorType::RGBA,

                cs => {
                    return Err(ImageErrors::EncodeErrors(
                        ImgEncodeErrors::UnsupportedColorspace(cs, self.supported_colorspaces()),
                    ));
                }
            },
            match image.depth() {
                BitDepth::Eight => oxipng::BitDepth::Eight,
                BitDepth::Sixteen => oxipng::BitDepth::Sixteen,
                d => {
                    return Err(ImageErrors::EncodeErrors(ImgEncodeErrors::Generic(
                        format!("{d:?} is not supported"),
                    )));
                }
            },
            data,
        )
        .map_err(|e| ImgEncodeErrors::ImageEncodeErrors(e.to_string()))?;

        #[cfg(feature = "metadata")]
        {
            if let Some(icc) = &image.metadata().icc_chunk() {
                img.add_icc_profile(icc);
            }
        }

        let mut writer = ZWriter::new(sink);

        let result = img
            .create_optimized_png(&self.options)
            .map_err(|e| ImgEncodeErrors::ImageEncodeErrors(e.to_string()))?;

        writer.write(&result).map_err(|e| {
            ImageErrors::EncodeErrors(ImgEncodeErrors::ImageEncodeErrors(format!("{e:?}")))
        })?;

        Ok(writer.bytes_written())
    }

    fn supported_colorspaces(&self) -> &'static [ColorSpace] {
        static SUPPORTED_COLORSPACES: [ColorSpace; 4] = [
            ColorSpace::Luma,
            ColorSpace::LumaA,
            ColorSpace::RGB,
            ColorSpace::RGBA,
        ];
        &SUPPORTED_COLORSPACES[..]
    }

    fn format(&self) -> ImageFormat {
        ImageFormat::PNG
    }

    fn supported_bit_depth(&self) -> &'static [BitDepth] {
        static SUPPORTED_BIT_DEPTHS: [BitDepth; 2] = [BitDepth::Eight, BitDepth::Sixteen];
        &SUPPORTED_BIT_DEPTHS[..]
    }

    fn default_depth(&self, depth: BitDepth) -> BitDepth {
        match depth {
            BitDepth::Sixteen | BitDepth::Float32 => BitDepth::Sixteen,
            _ => BitDepth::Eight,
        }
    }
}

/// Build the encode error for a dimension that does not fit in PNG's u32.
fn dimension_overflow(width: usize, height: usize) -> ImageErrors {
    ImageErrors::EncodeErrors(ImgEncodeErrors::ImageEncodeErrors(format!(
        "image dimensions {width}x{height} exceed the PNG u32 limit"
    )))
}

#[cfg(test)]
mod tests;
