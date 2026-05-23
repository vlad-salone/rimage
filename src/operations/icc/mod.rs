use lcms2::*;
use std::sync::Mutex;
use zune_core::{bit_depth::BitType, colorspace::ColorSpace};
use zune_image::{
    errors::{ImageErrors, ImageOperationsErrors},
    frame::Frame,
    image::Image,
    traits::OperationsTrait,
};

/// Apply icc profile
pub struct ApplyICC {
    profile: Mutex<Profile<GlobalContext>>,
}

impl ApplyICC {
    /// Create a new icc apply operation
    ///
    /// # Arguments
    /// - profile: ICC profile
    #[must_use]
    pub fn new(profile: Profile<GlobalContext>) -> Self {
        Self {
            profile: Mutex::new(profile),
        }
    }
}

impl OperationsTrait for ApplyICC {
    fn name(&self) -> &'static str {
        "apply icc profile"
    }

    fn execute_impl(&self, image: &mut Image) -> Result<(), ImageErrors> {
        let src_profile = match image.metadata().icc_chunk() {
            Some(icc) => Profile::new_icc(icc)
                .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?,
            None => Profile::new_srgb(),
        };

        let colorspace = image.colorspace();

        let format = match (colorspace, image.depth().bit_size()) {
            (ColorSpace::RGB, 8) => PixelFormat::RGB_8,
            (ColorSpace::RGB, 16) => PixelFormat::RGB_16,
            (ColorSpace::RGBA, 8) => PixelFormat::RGBA_8,
            (ColorSpace::RGBA, 16) => PixelFormat::RGBA_16,
            (ColorSpace::YCbCr, 8) => PixelFormat::YCbCr_8,
            (ColorSpace::YCbCr, 16) => PixelFormat::YCbCr_16,
            (ColorSpace::Luma, 8) => PixelFormat::GRAY_8,
            (ColorSpace::Luma, 16) => PixelFormat::GRAY_16,
            (ColorSpace::LumaA, 8) => PixelFormat::GRAYA_8,
            (ColorSpace::LumaA, 16) => PixelFormat::GRAYA_16,
            (ColorSpace::CMYK, 8) => PixelFormat::CMYK_8,
            (ColorSpace::CMYK, 16) => PixelFormat::CMYK_16,
            (ColorSpace::BGR, 8) => PixelFormat::BGR_8,
            (ColorSpace::BGR, 16) => PixelFormat::BGR_16,
            (ColorSpace::BGRA, 8) => PixelFormat::BGRA_8,
            (ColorSpace::BGRA, 16) => PixelFormat::BGRA_16,
            (ColorSpace::ARGB, 8) => PixelFormat::ARGB_8,
            (ColorSpace::ARGB, 16) => PixelFormat::ARGB_16,
            (ColorSpace::HSV, 8) => PixelFormat::HSV_8,
            (ColorSpace::HSV, 16) => PixelFormat::HSV_16,
            (cs, depth) => {
                return Err(ImageErrors::OperationsError(
                    ImageOperationsErrors::GenericString(format!(
                        "ICC profile application not supported for {cs:?} at {depth}-bit",
                    )),
                ));
            }
        };

        let profile = self
            .profile
            .lock()
            .map_err(|_| ImageOperationsErrors::GenericString("Mutex poisoned".to_string()))?;

        let t = Transform::new_flags(
            &src_profile,
            format,
            &profile,
            format,
            Intent::Perceptual,
            Flags::NO_CACHE,
        )
        .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?;

        for frame in image.frames_mut() {
            let mut buffer = frame.flatten::<u8>();
            t.transform_in_place(&mut buffer);
            let _ = std::mem::replace(frame, Frame::from_u8(&buffer, colorspace, 0, 0));
        }

        image.metadata_mut().set_icc_chunk(
            profile
                .icc()
                .map_err(|e| ImageOperationsErrors::GenericString(e.to_string()))?,
        );

        Ok(())
    }

    fn supported_types(&self) -> &'static [BitType] {
        &[BitType::U8, BitType::U16]
    }

    fn supported_colorspaces(&self) -> &'static [ColorSpace] {
        &[
            ColorSpace::RGB,
            ColorSpace::RGBA,
            ColorSpace::YCbCr,
            ColorSpace::Luma,
            ColorSpace::LumaA,
            ColorSpace::CMYK,
            ColorSpace::BGR,
            ColorSpace::BGRA,
            ColorSpace::ARGB,
            ColorSpace::HSV,
        ]
    }
}

/// Apply srgb icc profile
pub struct ApplySRGB;

impl OperationsTrait for ApplySRGB {
    fn name(&self) -> &'static str {
        "apply srgb profile"
    }

    fn execute_impl(&self, image: &mut Image) -> Result<(), ImageErrors> {
        if image.metadata().icc_chunk().is_none() {
            log::warn!("No icc profile in the image, skipping");
            return Ok(());
        }

        ApplyICC::new(Profile::new_srgb()).execute_impl(image)?;

        Ok(())
    }

    fn supported_types(&self) -> &'static [BitType] {
        &[BitType::U8, BitType::U16]
    }

    fn supported_colorspaces(&self) -> &'static [ColorSpace] {
        &[
            ColorSpace::RGB,
            ColorSpace::RGBA,
            ColorSpace::YCbCr,
            ColorSpace::Luma,
            ColorSpace::LumaA,
            ColorSpace::CMYK,
            ColorSpace::BGR,
            ColorSpace::BGRA,
            ColorSpace::ARGB,
            ColorSpace::HSV,
        ]
    }
}

#[cfg(test)]
mod tests;
