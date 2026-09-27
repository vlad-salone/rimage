use std::io::{Read, Seek};

use zune_core::colorspace::ColorSpace;
use zune_image::{errors::ImageErrors, image::Image, traits::DecoderTrait};

/// Upper bound on the pixel count of a TIFF this decoder will accept.
///
/// The CLI pre-checks dimensions via `limits`, but the library entry point is
/// reachable directly. A file that declares gigantic dimensions would make
/// `read_image` allocate them before the tiff crate's own checks run. 100 MP
/// is generous for photography and blocks the gigapixel OOM attack.
const MAX_TIFF_PIXELS: u64 = 100_000_000;

/// A Tiff decoder
pub struct TiffDecoder<R: Read + Seek> {
    inner: tiff::decoder::Decoder<R>,
    dimensions: Option<(usize, usize)>,
    colorspace: ColorSpace,
}

impl<R: Read + Seek> TiffDecoder<R> {
    /// Create a new tiff decoder that reads data from `source`
    pub fn try_new(source: R) -> Result<Self, ImageErrors> {
        let inner = tiff::decoder::Decoder::new(source).map_err(|e| {
            ImageErrors::ImageDecodeErrors(format!("Unable to create TIFF decoder: {e}"))
        })?;

        Ok(Self {
            inner,
            dimensions: None,
            colorspace: ColorSpace::Unknown,
        })
    }
}

impl<R> DecoderTrait for TiffDecoder<R>
where
    R: Read + Seek,
{
    fn decode(&mut self) -> Result<Image, ImageErrors> {
        let (width, height) = self.inner.dimensions().map_err(|e| {
            ImageErrors::ImageDecodeErrors(format!("Unable to read dimensions - {e}"))
        })?;

        let (width, height) = (width as usize, height as usize);

        // Reject an oversized image before `read_image` allocates it. The
        // tiff crate trusts the declared dimensions.
        let pixels = (width as u64).checked_mul(height as u64);
        match pixels {
            Some(p) if p <= MAX_TIFF_PIXELS => {}
            _ => {
                return Err(ImageErrors::ImageDecodeErrors(format!(
                    "TIFF dimensions {width}x{height} exceed the {MAX_TIFF_PIXELS} pixel limit"
                )));
            }
        }

        self.dimensions = Some((width, height));

        let colortype = self.inner.colortype().map_err(|e| {
            ImageErrors::ImageDecodeErrors(format!("Unable to read colorspace - {e}"))
        })?;
        let colorspace = match colortype {
            tiff::ColorType::RGB(_) => ColorSpace::RGB,
            tiff::ColorType::RGBA(_) => ColorSpace::RGBA,
            tiff::ColorType::CMYK(_) => ColorSpace::CMYK,
            tiff::ColorType::Gray(_) => ColorSpace::Luma,
            tiff::ColorType::GrayA(_) => ColorSpace::LumaA,
            tiff::ColorType::YCbCr(_) => ColorSpace::YCbCr,
            other => {
                return Err(ImageErrors::ImageDecodeErrors(format!(
                    "Unsupported TIFF color type: {other:?}"
                )));
            }
        };

        self.colorspace = colorspace;

        let result = self.inner.read_image().map_err(|e| {
            ImageErrors::ImageDecodeErrors(format!("Unable to decode TIFF file - {e}"))
        })?;

        match result {
            tiff::decoder::DecodingResult::U8(data) => {
                Ok(Image::from_u8(&data, width, height, colorspace))
            }
            tiff::decoder::DecodingResult::U16(data) => {
                Ok(Image::from_u16(&data, width, height, colorspace))
            }
            tiff::decoder::DecodingResult::F32(data) => {
                Ok(Image::from_f32(&data, width, height, colorspace))
            }
            _ => Err(ImageErrors::ImageDecodeErrors(
                "Tiff Data format not supported".to_string(),
            )),
        }
    }

    fn dimensions(&self) -> Option<(usize, usize)> {
        self.dimensions
    }

    fn out_colorspace(&self) -> ColorSpace {
        self.colorspace
    }

    fn name(&self) -> &'static str {
        "tiff-decoder"
    }
}

#[cfg(test)]
mod tests;
