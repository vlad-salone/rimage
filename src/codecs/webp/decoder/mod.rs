use std::{io::Read, marker::PhantomData};

use webp::{AnimDecoder, DecodeAnimImage};
use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace};
use zune_image::{errors::ImageErrors, frame::Frame, image::Image, traits::DecoderTrait};

/// A WebP decoder
pub struct WebPDecoder<R: Read> {
    inner: DecodeAnimImage,
    phantom: PhantomData<R>,
}

impl<R: Read> WebPDecoder<R> {
    /// Create a new webp decoder that reads data from `source`
    pub fn try_new(mut source: R) -> Result<WebPDecoder<R>, ImageErrors> {
        let mut buf = Vec::new();
        source.read_to_end(&mut buf)?;

        let decoder = AnimDecoder::new(&buf);
        let img = decoder.decode().map_err(ImageErrors::ImageDecodeErrors)?;

        Ok(WebPDecoder {
            inner: img,
            phantom: PhantomData,
        })
    }
}

impl<R> DecoderTrait for WebPDecoder<R>
where
    R: Read,
{
    fn decode(&mut self) -> Result<Image, ImageErrors> {
        let (width, height) = self.dimensions().ok_or_else(|| {
            ImageErrors::ImageDecodeErrors("WebP image has no frames".to_string())
        })?;
        let color = self.out_colorspace();

        let frames = self
            .inner
            .into_iter()
            .enumerate()
            .map(|(idx, frame)| {
                Frame::from_u8(frame.get_image(), color, idx, frame.get_time_ms() as usize)
            })
            .collect::<Vec<_>>();

        if frames.is_empty() {
            return Err(ImageErrors::ImageDecodeErrors(
                "WebP image contains no frames".to_string(),
            ));
        }

        Ok(Image::new_frames(
            frames,
            BitDepth::Eight,
            width,
            height,
            color,
        ))
    }

    fn dimensions(&self) -> Option<(usize, usize)> {
        let frame = self.inner.get_frame(0)?;

        Some((frame.width() as usize, frame.height() as usize))
    }

    fn out_colorspace(&self) -> ColorSpace {
        self.inner
            .get_frame(0)
            .map(|frame| match frame.get_layout() {
                webp::PixelLayout::Rgb => ColorSpace::RGB,
                webp::PixelLayout::Rgba => ColorSpace::RGBA,
            })
            .unwrap_or(ColorSpace::RGBA)
    }

    fn name(&self) -> &'static str {
        "webp"
    }
}

#[cfg(test)]
mod tests;
