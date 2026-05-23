use std::io::{Seek, SeekFrom};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use clap::ArgMatches;
#[cfg(feature = "avif")]
use rimage::codecs::avif::AvifEncoder;
#[cfg(feature = "mozjpeg")]
use rimage::codecs::mozjpeg::MozJpegEncoder;
#[cfg(feature = "oxipng")]
use rimage::codecs::oxipng::OxiPngEncoder;
#[cfg(feature = "webp")]
use rimage::codecs::webp::WebPEncoder;
use zune_core::{bytestream::ZByteWriterTrait, options::EncoderOptions};
use zune_image::{
    codecs::{
        ImageFormat, farbfeld::FarbFeldEncoder, jpeg::JpegEncoder, jpeg_xl::JxlEncoder,
        png::PngEncoder, ppm::PPMEncoder, qoi::QoiEncoder,
    },
    errors::ImageErrors,
    image::Image,
    metadata::AlphaState,
    traits::{EncoderTrait, OperationsTrait},
};
use zune_imageprocs::premul_alpha::PremultiplyAlpha;

pub fn decode<P: AsRef<Path>>(f: P) -> Result<Image, ImageErrors> {
    Image::open(f.as_ref()).or_else(|e| {
        if matches!(e, ImageErrors::ImageDecoderNotImplemented(_)) {
            #[cfg(any(feature = "avif", feature = "webp"))]
            let mut file = File::open(f.as_ref())?;

            #[cfg(feature = "avif")]
            {
                let mut file_content = vec![];

                file.read_to_end(&mut file_content)?;
                file.seek(SeekFrom::Start(0))?;

                if libavif::is_avif(&file_content) {
                    use rimage::codecs::avif::AvifDecoder;

                    let decoder = AvifDecoder::try_new(file)?;

                    return Image::from_decoder(decoder);
                };
                file.seek(SeekFrom::Start(0))?;
            }

            #[cfg(feature = "webp")]
            {
                if f.as_ref()
                    .extension()
                    .is_some_and(|f| f.eq_ignore_ascii_case("webp"))
                {
                    use rimage::codecs::webp::WebPDecoder;

                    let decoder = WebPDecoder::try_new(file)?;

                    return Image::from_decoder(decoder);
                }

                file.seek(SeekFrom::Start(0))?;
            }

            #[cfg(feature = "tiff")]
            {
                if f.as_ref()
                    .extension()
                    .is_some_and(|f| f.eq_ignore_ascii_case("tiff") | f.eq_ignore_ascii_case("tif"))
                {
                    use rimage::codecs::tiff::TiffDecoder;

                    let decoder = TiffDecoder::try_new(file)?;

                    return Image::from_decoder(decoder);
                }

                file.seek(SeekFrom::Start(0))?;
            }

            Err(ImageErrors::ImageDecoderNotImplemented(
                ImageFormat::Unknown,
            ))
        } else {
            Err(e)
        }
    })
}

#[allow(unused_variables)]
#[allow(unused_mut)]
pub fn operations(matches: &ArgMatches, img: &Image) -> BTreeMap<usize, Box<dyn OperationsTrait>> {
    let mut map: BTreeMap<usize, Box<dyn OperationsTrait>> = BTreeMap::new();

    #[cfg(feature = "resize")]
    {
        use crate::cli::preprocessors::{ResizeFilter, ResizeValue};
        use fast_image_resize::ResizeAlg;
        use rimage::operations::resize::Resize;

        if let Some(values) = matches.get_many::<ResizeValue>("resize") {
            let filter = matches.get_one::<ResizeFilter>("filter");

            let downscale = matches.get_flag("downscale") && !matches.get_flag("no-downscale");
            let upscale = matches.get_flag("upscale") && !matches.get_flag("no-upscale");

            log::debug!("downscale: {downscale}, upscale: {upscale}");

            let size = img.dimensions();

            values
                .into_iter()
                .zip(matches.indices_of("resize").unwrap())
                .for_each(|(value, idx)| {
                    let (w, h) = value.map_dimensions(size.0, size.1);
                    log::trace!("setup resize {value} on index {idx}");

                    // Skip if the image is already the desired size
                    // or if both downscale and upscale are disabled
                    if (!downscale && !upscale) || (w == size.0 && h == size.1) {
                        log::trace!("skip resize to {w}x{h} on index {idx}");
                        return;
                    }

                    if !downscale && (w <= size.0 || h <= size.1) {
                        log::trace!("downscaling disabled, skip resize to {w}x{h} on index {idx}");
                        return;
                    }

                    if !upscale && (w >= size.0 || h >= size.1) {
                        log::trace!("upscaling disabled, skip resize to {w}x{h} on index {idx}");
                        return;
                    }

                    map.insert(
                        idx,
                        Box::new(Resize::new(
                            w,
                            h,
                            filter
                                .copied()
                                .map(Into::<ResizeAlg>::into)
                                .unwrap_or_default(),
                        )),
                    );
                })
        }
    }

    #[cfg(feature = "quantization")]
    {
        use rimage::operations::quantize::Quantize;

        if let Some(values) = matches.get_many::<u8>("quantization") {
            let dithering = matches.get_one::<u8>("dithering");

            values
                .into_iter()
                .zip(matches.indices_of("quantization").unwrap())
                .for_each(|(value, idx)| {
                    log::trace!("setup quantization {value} on index {idx}");

                    map.insert(
                        idx,
                        Box::new(Quantize::new(*value, dithering.map(|q| *q as f32 / 100.))),
                    );
                })
        }
    }

    if let Some(values) = matches.get_many::<bool>("premultiply") {
        values
            .into_iter()
            .zip(matches.indices_of("premultiply").unwrap())
            .for_each(|(value, idx)| {
                if let Some(op) = map.get(&(idx + 2)) {
                    log::trace!("setup alpha premultiply for {}", op.name());

                    map.insert(
                        idx,
                        Box::new(PremultiplyAlpha::new(AlphaState::PreMultiplied)),
                    );

                    assert!(
                        !map.contains_key(&(idx + 3)),
                        "There is a operation at {} aborting",
                        idx + 3
                    );

                    map.insert(
                        idx + 3,
                        Box::new(PremultiplyAlpha::new(AlphaState::NonPreMultiplied)),
                    );
                } else {
                    log::warn!("No operation found for premultiply at index {idx}")
                }
            })
    }

    map
}

pub enum AvailableEncoders {
    FarbFeld(Box<FarbFeldEncoder>),
    Jpeg(Box<JpegEncoder>),
    JpegXl(Box<JxlEncoder>),
    #[cfg(feature = "mozjpeg")]
    MozJpeg(Box<MozJpegEncoder>),
    #[cfg(feature = "oxipng")]
    OxiPng(Box<OxiPngEncoder>),
    #[cfg(feature = "avif")]
    Avif(Box<AvifEncoder>),
    #[cfg(feature = "webp")]
    Webp(Box<WebPEncoder>),
    Png(Box<PngEncoder>),
    Ppm(Box<PPMEncoder>),
    Qoi(Box<QoiEncoder>),
}

impl AvailableEncoders {
    pub fn to_extension(&self) -> &'static str {
        match self {
            AvailableEncoders::FarbFeld(_) => "ff",
            AvailableEncoders::Jpeg(_) => "jpg",
            AvailableEncoders::JpegXl(_) => "jxl",
            #[cfg(feature = "mozjpeg")]
            AvailableEncoders::MozJpeg(_) => "jpg",
            #[cfg(feature = "oxipng")]
            AvailableEncoders::OxiPng(_) => "png",
            #[cfg(feature = "avif")]
            AvailableEncoders::Avif(_) => "avif",
            #[cfg(feature = "webp")]
            AvailableEncoders::Webp(_) => "webp",
            AvailableEncoders::Png(_) => "png",
            AvailableEncoders::Ppm(_) => "ppm",
            AvailableEncoders::Qoi(_) => "qoi",
        }
    }

    pub fn encode<T: ZByteWriterTrait>(
        &mut self,
        img: &Image,
        sink: T,
    ) -> Result<usize, ImageErrors> {
        match self {
            AvailableEncoders::FarbFeld(enc) => enc.encode(img, sink),
            AvailableEncoders::Jpeg(enc) => enc.encode(img, sink),
            AvailableEncoders::JpegXl(enc) => enc.encode(img, sink),
            AvailableEncoders::MozJpeg(enc) => enc.encode(img, sink),
            AvailableEncoders::OxiPng(enc) => enc.encode(img, sink),
            AvailableEncoders::Avif(enc) => enc.encode(img, sink),
            AvailableEncoders::Webp(enc) => enc.encode(img, sink),
            AvailableEncoders::Png(enc) => enc.encode(img, sink),
            AvailableEncoders::Ppm(enc) => enc.encode(img, sink),
            AvailableEncoders::Qoi(enc) => enc.encode(img, sink),
        }
    }
}

pub fn encoder(name: &str, matches: &ArgMatches) -> Result<AvailableEncoders, ImageErrors> {
    match name {
        "farbfeld" => Ok(AvailableEncoders::FarbFeld(
            Box::new(FarbFeldEncoder::new()),
        )),
        "jpeg" => {
            let options = EncoderOptions::default();

            if let Some(quality) = matches.get_one::<u8>("quality") {
                options.set_quality(*quality);
            }

            options.set_jpeg_encode_progressive(matches.get_flag("progressive"));

            Ok(AvailableEncoders::Jpeg(Box::new(
                JpegEncoder::new_with_options(options),
            )))
        }
        "jpeg_xl" => Ok(AvailableEncoders::JpegXl(Box::new(JxlEncoder::new()))),
        #[cfg(feature = "mozjpeg")]
        "mozjpeg" => {
            use mozjpeg::qtable;
            use rimage::codecs::mozjpeg::MozJpegOptions;

            let quality = matches.get_one::<u8>("quality").copied().unwrap_or(75) as f32;
            let chroma_quality = matches
                .get_one::<u8>("chroma_quality")
                .map(|q| *q as f32)
                .unwrap_or(quality);

            let options = MozJpegOptions {
                quality,
                progressive: !matches.get_flag("baseline"),
                optimize_coding: !matches.get_flag("no_optimize_coding"),
                smoothing: matches
                    .get_one::<u8>("smoothing")
                    .copied()
                    .unwrap_or_default(),
                color_space: match matches
                    .get_one::<String>("colorspace")
                    .map(|s| s.as_str())
                    .unwrap_or("ycbcr")
                {
                    "ycbcr" => mozjpeg::ColorSpace::JCS_YCbCr,
                    "rgb" => mozjpeg::ColorSpace::JCS_EXT_RGB,
                    "grayscale" => mozjpeg::ColorSpace::JCS_GRAYSCALE,
                    cs => {
                        return Err(ImageErrors::GenericString(format!(
                            "Unsupported mozjpeg colorspace: {cs}",
                        )));
                    }
                },
                trellis_multipass: matches.get_flag("multipass"),
                chroma_subsample: matches.get_one::<u8>("subsample").copied(),

                luma_qtable: match matches.get_one::<String>("qtable") {
                    Some(c) => Some(match c.as_str() {
                        "AhumadaWatsonPeterson" => {
                            qtable::AhumadaWatsonPeterson.scaled(quality, quality)
                        }
                        "AnnexK" => qtable::AnnexK_Luma.scaled(quality, quality),
                        "Flat" => qtable::Flat.scaled(quality, quality),
                        "KleinSilversteinCarney" => {
                            qtable::KleinSilversteinCarney.scaled(quality, quality)
                        }
                        "MSSSIM" => qtable::MSSSIM_Luma.scaled(quality, quality),
                        "NRobidoux" => qtable::NRobidoux.scaled(quality, quality),
                        "PSNRHVS" => qtable::PSNRHVS_Luma.scaled(quality, quality),
                        "PetersonAhumadaWatson" => {
                            qtable::PetersonAhumadaWatson.scaled(quality, quality)
                        }
                        "WatsonTaylorBorthwick" => {
                            qtable::WatsonTaylorBorthwick.scaled(quality, quality)
                        }
                        q => {
                            return Err(ImageErrors::GenericString(
                                format!("Unknown qtable: {q}",),
                            ));
                        }
                    }),
                    None => None,
                },

                chroma_qtable: match matches.get_one::<String>("qtable") {
                    Some(c) => Some(match c.as_str() {
                        "AhumadaWatsonPeterson" => {
                            qtable::AhumadaWatsonPeterson.scaled(chroma_quality, chroma_quality)
                        }
                        "AnnexK" => qtable::AnnexK_Chroma.scaled(chroma_quality, chroma_quality),
                        "Flat" => qtable::Flat.scaled(chroma_quality, chroma_quality),
                        "KleinSilversteinCarney" => {
                            qtable::KleinSilversteinCarney.scaled(chroma_quality, chroma_quality)
                        }
                        "MSSSIM" => qtable::MSSSIM_Chroma.scaled(chroma_quality, chroma_quality),
                        "NRobidoux" => qtable::NRobidoux.scaled(chroma_quality, chroma_quality),
                        "PSNRHVS" => qtable::PSNRHVS_Chroma.scaled(chroma_quality, chroma_quality),
                        "PetersonAhumadaWatson" => {
                            qtable::PetersonAhumadaWatson.scaled(chroma_quality, chroma_quality)
                        }
                        "WatsonTaylorBorthwick" => {
                            qtable::WatsonTaylorBorthwick.scaled(chroma_quality, chroma_quality)
                        }
                        q => {
                            return Err(ImageErrors::GenericString(
                                format!("Unknown qtable: {q}",),
                            ));
                        }
                    }),
                    None => None,
                },
            };

            Ok(AvailableEncoders::MozJpeg(Box::new(
                MozJpegEncoder::new_with_options(options),
            )))
        }
        #[cfg(feature = "oxipng")]
        "oxipng" => {
            use rimage::codecs::oxipng::OxiPngOptions;

            let mut options =
                OxiPngOptions::from_preset(*matches.get_one::<u8>("effort").unwrap_or(&2));

            options.interlace = if matches.get_flag("interlace") {
                Some(true)
            } else {
                None
            };

            Ok(AvailableEncoders::OxiPng(Box::new(
                OxiPngEncoder::new_with_options(options),
            )))
        }
        #[cfg(feature = "avif")]
        "avif" => {
            use rimage::codecs::avif::AvifOptions;

            let options = AvifOptions {
                quality: matches.get_one::<u8>("quality").copied().unwrap_or(50) as f32,
                alpha_quality: matches.get_one::<u8>("alpha_quality").map(|q| *q as f32),
                speed: matches.get_one::<u8>("speed").copied().unwrap_or(6),
                color_space: match matches
                    .get_one::<String>("colorspace")
                    .map(|s| s.as_str())
                    .unwrap_or("ycbcr")
                {
                    "ycbcr" => ravif::ColorModel::YCbCr,
                    "rgb" => ravif::ColorModel::RGB,
                    cs => {
                        return Err(ImageErrors::GenericString(format!(
                            "Unsupported avif colorspace: {cs}",
                        )));
                    }
                },
                alpha_color_mode: match matches
                    .get_one::<String>("alpha_mode")
                    .map(|s| s.as_str())
                    .unwrap_or("UnassociatedClean")
                {
                    "UnassociatedDirty" => ravif::AlphaColorMode::UnassociatedDirty,
                    "UnassociatedClean" => ravif::AlphaColorMode::UnassociatedClean,
                    "Premultiplied" => ravif::AlphaColorMode::Premultiplied,
                    mode => {
                        return Err(ImageErrors::GenericString(format!(
                            "Unsupported avif alpha mode: {mode}",
                        )));
                    }
                },
            };

            Ok(AvailableEncoders::Avif(Box::new(
                AvifEncoder::new_with_options(options),
            )))
        }
        #[cfg(feature = "webp")]
        "webp" => {
            use rimage::codecs::webp::WebPOptions;

            let mut options = WebPOptions::new().unwrap();

            options.quality = matches.get_one::<u8>("quality").copied().unwrap_or(75) as f32;
            options.lossless = matches.get_flag("lossless") as i32;
            options.near_lossless =
                100 - matches.get_one::<u8>("slight_loss").copied().unwrap_or(0) as i32;
            options.exact = matches.get_flag("exact") as i32;

            Ok(AvailableEncoders::Webp(Box::new(
                WebPEncoder::new_with_options(options),
            )))
        }
        "png" => Ok(AvailableEncoders::Png(Box::new(PngEncoder::new()))),
        "ppm" => Ok(AvailableEncoders::Ppm(Box::new(PPMEncoder::new()))),
        "qoi" => Ok(AvailableEncoders::Qoi(Box::new(QoiEncoder::new()))),

        name => Err(ImageErrors::GenericString(format!(
            "Encoder \"{name}\" not found",
        ))),
    }
}
