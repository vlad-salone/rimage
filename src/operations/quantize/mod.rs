use imagequant::Histogram;
use rgb::FromSlice;
use zune_core::{bit_depth::BitType, colorspace::ColorSpace};
use zune_image::{
    channel::Channel,
    errors::{ImageErrors, ImageOperationsErrors},
    traits::OperationsTrait,
};

/// Reduce image palette
pub struct Quantize {
    quality: u8,
    dithering: Option<f32>,
}

impl Quantize {
    /// Create a new quantization operation
    ///
    /// # Arguments
    /// - quality: resulting quality of the palette
    /// - dithering: overall "smoothness" of the resulting image
    #[must_use]
    pub fn new(quality: u8, dithering: Option<f32>) -> Self {
        Self { quality, dithering }
    }
}

impl OperationsTrait for Quantize {
    fn name(&self) -> &'static str {
        "quantize"
    }

    fn execute_impl(&self, image: &mut zune_image::image::Image) -> Result<(), ImageErrors> {
        let (src_width, src_height) = image.dimensions();
        let channel_len = src_width * src_height;

        let mut liq = imagequant::new();

        liq.set_quality(0, self.quality)
            .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;

        let mut histogram = Histogram::new(&liq);

        let mut frames = image
            .frames_ref()
            .iter()
            .map(|frame| {
                let mut img = liq
                    .new_image(frame.flatten().as_rgba(), src_width, src_height, 0.0)
                    .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;

                histogram
                    .add_image(&liq, &mut img)
                    .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;

                Ok::<imagequant::Image, ImageErrors>(img)
            })
            .collect::<Result<Vec<imagequant::Image>, ImageErrors>>()?;

        let mut res = histogram
            .quantize(&liq)
            .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;

        if let Some(dithering) = self.dithering {
            res.set_dithering_level(dithering)
                .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;
        }

        frames
            .iter_mut()
            .zip(image.frames_mut())
            .try_for_each(|(img, frame)| {
                let (palette, pixels) = res
                    .remapped(img)
                    .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;

                let channels = pixels
                    .iter()
                    .enumerate()
                    .map(|(idx, raw_px)| {
                        let px = palette[*raw_px as usize];
                        (idx, px.r, px.g, px.b, px.a)
                    })
                    .fold(
                        vec![Channel::new_with_bit_type(channel_len, BitType::U8); 4],
                        |mut acc, (idx, r, g, b, a)| {
                            // SAFETY: idx is bounded by pixels.len() which equals
                            // src_width * src_height, and channels are pre-allocated
                            // with channel_len (= src_width * src_height) bytes of U8.
                            // Each alias_mut slice therefore has at least idx+1 elements.
                            unsafe {
                                debug_assert!(idx < channel_len);
                                acc[0].alias_mut()[idx] = r;
                                acc[1].alias_mut()[idx] = g;
                                acc[2].alias_mut()[idx] = b;
                                acc[3].alias_mut()[idx] = a;
                            }

                            acc
                        },
                    );

                frame.set_channels(channels);

                Ok::<(), ImageErrors>(())
            })?;

        Ok(())
    }

    fn supported_types(&self) -> &'static [BitType] {
        &[BitType::U8][..]
    }

    fn supported_colorspaces(&self) -> &'static [ColorSpace] {
        &[ColorSpace::RGBA][..]
    }
}

#[cfg(test)]
mod tests;
