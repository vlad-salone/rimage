use std::fs::File;

use super::*;

#[test]
fn decode() {
    let file_content = File::open("tests/files/webp/f1t.webp").unwrap();

    let decoder = WebPDecoder::try_new(file_content).unwrap();

    let img = Image::from_decoder(decoder).unwrap();

    assert_eq!(img.dimensions(), (48, 80));
    // This file has no alpha channel, and the static decoder reports the layout
    // it actually produced rather than assuming RGBA.
    assert_eq!(img.colorspace(), ColorSpace::RGB);
}

/// A file without alpha must not be silently widened to four channels: doing so
/// allocates a third more memory than the image needs.
#[test]
fn a_file_without_alpha_stays_rgb() {
    let file_content = File::open("tests/files/webp/f1t.webp").unwrap();

    let decoder = WebPDecoder::try_new(file_content).unwrap();

    assert_eq!(decoder.out_colorspace(), ColorSpace::RGB);
    let (width, height) = decoder.dimensions().unwrap();
    assert_eq!((width, height), (48, 80));
}

/// A still file is served by the non-animated decoder, which is the whole point
/// of the change: libwebp's animation decoder materialises every frame, and the
/// static one never does.
///
/// The animated branch is not covered here because no animated WebP fixture is
/// committed and the shipped encoder writes only a still image. The dispatch
/// itself is asserted by checking which field was populated.
#[test]
fn a_still_file_takes_the_non_animated_path() {
    let file_content = File::open("tests/files/webp/f1t.webp").unwrap();

    let decoder = WebPDecoder::try_new(file_content).unwrap();

    assert!(
        decoder.still.is_some(),
        "a still WebP must be decoded by libwebp's static decoder"
    );
    assert!(
        decoder.animated.is_none(),
        "the animation decoder must not run for a still image"
    );
}

/// Animated and corrupt inputs both make `Decoder::decode` return `None`, so the
/// animation decoder has to be the fallback that reports *why* the file failed.
#[test]
fn an_undecodable_file_reports_the_animation_decoder_error() {
    let garbage = vec![0u8; 32];

    let Err(error) = WebPDecoder::try_new(garbage.as_slice()) else {
        panic!("garbage must not decode");
    };

    assert!(!error.to_string().is_empty());
}
