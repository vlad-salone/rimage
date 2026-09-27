use std::{io::Read, marker::PhantomData};

use webp::{AnimDecoder, DecodeAnimImage, Decoder, WebPImage};
use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::DecoderOptions};
use zune_image::{errors::ImageErrors, frame::Frame, image::Image, traits::DecoderTrait};

/// A WebP decoder
///
/// Only the first frame of an animated file is decoded. libwebp's animation
/// decoder materialises *every* frame before handing anything back, and this
/// program has no use for the rest: it re-encodes a single still image. On a
/// long animation that difference is the whole memory budget, so the static
/// decoder is preferred whenever it applies.
///
/// Note that `DecoderOptions` are accepted but barely apply here: libwebp
/// decides the output layout from the bitstream, and this decoder reports that
/// layout rather than converting. The parameter exists so the shared decode
/// entry point can pass one set of options to every decoder uniformly.
pub struct WebPDecoder<R: Read> {
    /// A still image decoded by libwebp's non-animated entry point.
    still: Option<WebPImage>,
    /// Animated path, used only when `still` is empty.
    animated: Option<DecodeAnimImage>,
    phantom: PhantomData<R>,
}

/// Upper bound on a WebP file read into memory before decoding.
///
/// libwebp has no file-size ceiling, and the static decoder path reads the
/// whole file before it can tell whether the bitstream is valid, so a hostile
/// multi-gigabyte "webp" would exhaust memory before any format check ran.
/// 256 MiB is far beyond any real WebP and matches the SVG decoder's cap.
const MAX_WEBP_BYTES: u64 = 256 * 1024 * 1024;

impl<R: Read> WebPDecoder<R> {
    /// Create a new webp decoder that reads data from `source`
    pub fn try_new(source: R) -> Result<WebPDecoder<R>, ImageErrors> {
        Self::try_new_with_options(source, DecoderOptions::default())
    }

    /// Create a new webp decoder with explicit [`DecoderOptions`].
    pub fn try_new_with_options(
        source: R,
        _options: DecoderOptions,
    ) -> Result<WebPDecoder<R>, ImageErrors> {
        let mut buf = Vec::new();
        source.take(MAX_WEBP_BYTES + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > MAX_WEBP_BYTES {
            return Err(ImageErrors::ImageDecodeErrors(format!(
                "WebP input exceeds the {} MiB read limit",
                MAX_WEBP_BYTES / 1024 / 1024
            )));
        }

        // `Decoder::decode` returns `None` for animated files as well as for
        // any other failure, so it cannot be the only path: it is the cheap
        // first attempt, and `AnimDecoder` is the fallback that also tells us
        // *why* something failed.
        if let Some(still) = Decoder::new(&buf).decode() {
            return Ok(WebPDecoder {
                still: Some(still),
                animated: None,
                phantom: PhantomData,
            });
        }

        let decoder = AnimDecoder::new(&buf);
        let img = decoder.decode().map_err(ImageErrors::ImageDecodeErrors)?;

        Ok(WebPDecoder {
            still: None,
            animated: Some(img),
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

        // A still image: one frame, no timestamp to reason about.
        if let Some(still) = self.still.take() {
            let frame = Frame::from_u8(&still, color, 1, 1);

            return Ok(Image::new_frames(
                vec![frame],
                BitDepth::Eight,
                width,
                height,
                color,
            ));
        }

        let animated = self.animated.take().ok_or_else(|| {
            ImageErrors::ImageDecodeErrors("WebP image has no frames".to_string())
        })?;

        // Only the first frame is needed, so the remaining frames are not
        // walked: `get_frame` gives one without collecting the rest.
        let first = animated.get_frame(0).ok_or_else(|| {
            ImageErrors::ImageDecodeErrors("WebP image contains no frames".to_string())
        })?;

        let frame = Frame::from_u8(first.get_image(), color, 1, 1);

        Ok(Image::new_frames(
            vec![frame],
            BitDepth::Eight,
            width,
            height,
            color,
        ))
    }

    fn dimensions(&self) -> Option<(usize, usize)> {
        if let Some(still) = &self.still {
            return Some((still.width() as usize, still.height() as usize));
        }

        let frame = self.animated.as_ref()?.get_frame(0)?;

        Some((frame.width() as usize, frame.height() as usize))
    }

    fn out_colorspace(&self) -> ColorSpace {
        let layout = match (&self.still, &self.animated) {
            (Some(still), _) => Some(still.layout()),
            (None, Some(animated)) => animated.get_frame(0).map(|frame| frame.get_layout()),
            (None, None) => None,
        };

        // libwebp chose the buffer layout, and `Frame::from_u8` reinterprets the
        // bytes with whichever colorspace we declare, so report the layout that
        // was actually produced rather than a guess.
        match layout {
            Some(webp::PixelLayout::Rgb) => ColorSpace::RGB,
            Some(webp::PixelLayout::Rgba) => ColorSpace::RGBA,
            None => ColorSpace::RGBA,
        }
    }

    fn name(&self) -> &'static str {
        "webp"
    }
}

#[cfg(test)]
mod tests;
