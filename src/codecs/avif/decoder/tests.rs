use std::{fs::File, io::Cursor};

use zune_image::traits::EncoderTrait;

use crate::codecs::avif::{AvifEncoder, AvifOptions};

use super::*;

#[test]
fn decode() {
    let file_content = File::open("tests/files/avif/f1t.avif").unwrap();

    let decoder = AvifDecoder::try_new(file_content).unwrap();

    let img = Image::from_decoder(decoder).unwrap();

    assert_eq!(img.dimensions(), (48, 80));
    assert_eq!(img.colorspace(), ColorSpace::RGBA);
}

#[test]
fn full_range_rgb_round_trip_preserves_color() {
    assert_color_round_trip(ColorSpace::RGB, [80, 140, 200, 255]);
    assert_color_round_trip(ColorSpace::RGB, [200, 140, 80, 255]);
}

#[test]
fn full_range_rgba_round_trip_preserves_color() {
    assert_color_round_trip(ColorSpace::RGBA, [80, 140, 200, 128]);
    assert_color_round_trip(ColorSpace::RGBA, [200, 140, 80, 128]);
}

fn assert_color_round_trip(colorspace: ColorSpace, expected: [u8; 4]) {
    let pixels = expected[..colorspace.num_components()].repeat(16 * 8);
    let image = Image::from_u8(&pixels, 16, 8, colorspace);
    let mut encoder = AvifEncoder::new_with_options(AvifOptions {
        quality: 100.0,
        ..Default::default()
    });
    let mut encoded = Vec::new();
    encoder.encode(&image, &mut encoded).unwrap();

    // Keep this regression on the high-depth, full-range conversion path.
    let parsed = avif_parse::read_avif(&mut encoded.as_slice()).unwrap();
    let picture = decode_av1_stream(&parsed.primary_item).unwrap();
    assert_eq!(picture.bits_per_component().unwrap().0, 10);
    assert_eq!(picture.color_range(), Dav1dYuvRange::Full);

    let decoder = AvifDecoder::try_new(Cursor::new(encoded)).unwrap();
    let decoded = Image::from_decoder(decoder).unwrap();
    assert_eq!(decoded.dimensions(), (16, 8));
    assert_eq!(decoded.colorspace(), ColorSpace::RGBA);
    for pixel in decoded.flatten_to_u8()[0].as_chunks::<4>().0 {
        for channel in 0..3 {
            assert!(
                pixel[channel].abs_diff(expected[channel]) <= 2,
                "expected {expected:?}, got {pixel:?}"
            );
        }
        assert!(pixel[3].abs_diff(expected[3]) <= 1, "alpha: {}", pixel[3]);
    }
}

#[test]
fn bt2020_constant_luminance_is_rejected() {
    assert!(color_transform(MatrixCoefficients::BT2020ConstantLuminance).is_err());
}

/// Packs samples into little-endian 16-bit rows; `stride` is in bytes.
fn u16_plane(samples: &[u16], stride: usize, rows: usize) -> Vec<u8> {
    let mut plane = vec![0u8; stride * rows];
    for (row, sample_row) in samples.chunks(stride / 2).take(rows).enumerate() {
        for (column, &sample) in sample_row.iter().enumerate() {
            let bytes = sample.to_le_bytes();
            plane[row * stride + column * 2] = bytes[0];
            plane[row * stride + column * 2 + 1] = bytes[1];
        }
    }
    plane
}

#[test]
fn high_depth_bt2020_limited() {
    // values taken from a real 10-bit sample; yuvutils-rs 0.8.3 mis-scales the
    // red channel for exactly this input (limited range, BT.2020, positive Cr)
    let y = u16_plane(&[834; 32], 16, 4);
    let u = u16_plane(&[491; 8], 8, 2);
    let v = u16_plane(&[586; 8], 8, 2);
    let planes = Planes {
        y: &y,
        y_stride: 16,
        u: &u,
        u_stride: 8,
        v: &v,
        v_stride: 8,
        width: 8,
        height: 4,
        layout: PixelLayout::I420,
    };

    let mut rgba = vec![0u8; 8 * 4 * 4];
    high_depth_ycbcr_to_rgba(
        &planes,
        10,
        false,
        &ColorTransform::Ycbcr {
            kr: 0.2627,
            kb: 0.0593,
            standard: YuvStandardMatrix::Bt2020,
        },
        &mut rgba,
    );

    // Y8=224, Cb8=-6.0, Cr8=+21.0 -> R clamps to 255, G=213, B=213
    for px in rgba.as_chunks::<4>().0 {
        assert_eq!(px[0], 255, "red");
        assert_eq!(px[1], 213, "green");
        assert_eq!(px[2], 213, "blue");
        assert_eq!(px[3], 255, "alpha");
    }
}

#[test]
fn twelve_bit_full_range_bt601_preserves_color() {
    // RGB (80, 140, 200) encoded as full-range, 12-bit BT.601 YCbCr.
    let y = u16_plane(&[2070; 4], 4, 2);
    let u = u16_plane(&[2692; 4], 4, 2);
    let v = u16_plane(&[1488; 4], 4, 2);
    let planes = Planes {
        y: &y,
        y_stride: 4,
        u: &u,
        u_stride: 4,
        v: &v,
        v_stride: 4,
        width: 2,
        height: 2,
        layout: PixelLayout::I444,
    };

    let mut rgba = [0u8; 16];
    high_depth_ycbcr_to_rgba(
        &planes,
        12,
        true,
        &ColorTransform::Ycbcr {
            kr: 0.299,
            kb: 0.114,
            standard: YuvStandardMatrix::Bt601,
        },
        &mut rgba,
    );

    for pixel in rgba.as_chunks::<4>().0 {
        assert_eq!(*pixel, [80, 140, 200, 255]);
    }
}

#[test]
fn high_depth_ycgco_round_trip() {
    // forward YCgCo (H.273 matrix 8) of R=200 G=50 B=100 in the 8-bit domain:
    // Y=100, Cg=-50, Co=+50. Scale each by 1023/255, add the 512 chroma
    // bias to Cg/Co, and round to the nearest integer.
    let y = u16_plane(&[401; 32], 16, 4);
    let u = u16_plane(&[311; 8], 8, 2);
    let v = u16_plane(&[713; 8], 8, 2);
    let planes = Planes {
        y: &y,
        y_stride: 16,
        u: &u,
        u_stride: 8,
        v: &v,
        v_stride: 8,
        width: 8,
        height: 4,
        layout: PixelLayout::I420,
    };

    let mut rgba = vec![0u8; 8 * 4 * 4];
    high_depth_ycbcr_to_rgba(&planes, 10, true, &ColorTransform::Ycgco, &mut rgba);

    for px in rgba.as_chunks::<4>().0 {
        assert!((px[0] as i32 - 200).abs() <= 2, "red: {}", px[0]);
        assert!((px[1] as i32 - 50).abs() <= 2, "green: {}", px[1]);
        assert!((px[2] as i32 - 100).abs() <= 2, "blue: {}", px[2]);
        assert_eq!(px[3], 255);
    }
}

#[test]
fn high_depth_luma_expansion_bounds() {
    // limited-range 10-bit luma spans [64, 940]
    assert_eq!(expand_luma(64, 10, false), 0.0);
    assert!((expand_luma(940, 10, false) - 255.0).abs() < 0.01);
    // full-range 10-bit spans [0, 1023]
    assert_eq!(expand_luma(0, 10, true), 0.0);
    assert!((expand_luma(1023, 10, true) - 255.0).abs() < 0.01);
    // limited-range chroma is neutral at 512
    assert_eq!(expand_chroma(512, 10, false), 0.0);
    // full-range chroma is neutral at 512
    assert_eq!(expand_chroma(512, 10, true), 0.0);
}

#[test]
fn full_range_chroma_expansion() {
    // Full-range chroma has a half-range bias, but a full-range scale:
    // H.273: C = (sample - 2^(bits - 1)) / (2^bits - 1), then scale by 255.
    for (bits, sample, expected) in [
        (10, 0, -127.624_63),
        (10, 256, -63.812_317),
        (10, 512, 0.0),
        (10, 768, 63.812_317),
        (10, 1023, 127.375_37),
        (12, 0, -127.531_136),
        (12, 1024, -63.765_568),
        (12, 2048, 0.0),
        (12, 3072, 63.765_568),
        (12, 4095, 127.468_864),
    ] {
        let actual = expand_chroma(sample, bits, true);
        assert!(
            (actual - expected).abs() < 0.001,
            "{bits}-bit sample {sample}: expected {expected}, got {actual}"
        );
    }
}

fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
    let mut box_data = vec![0u8; 8 + 4 + 4 + 4 * compatible.len()];
    let size = box_data.len() as u32;
    box_data[..4].copy_from_slice(&size.to_be_bytes());
    box_data[4..8].copy_from_slice(b"ftyp");
    box_data[8..12].copy_from_slice(major);
    for (index, brand) in compatible.iter().enumerate() {
        let start = 16 + index * 4;
        box_data[start..start + 4].copy_from_slice(brand.as_slice());
    }
    box_data
}

#[test]
fn avif_sniffing() {
    assert!(is_avif(&ftyp(b"avif", &[b"mif1"])));
    // `avis` (image sequences) as major brand
    assert!(is_avif(&ftyp(b"avis", &[b"mif1"])));
    // brand listed only in the compatible brands
    assert!(is_avif(&ftyp(b"mif1", &[b"avif"])));

    assert!(!is_avif(&ftyp(b"isom", &[b"mif1"])));
    assert!(!is_avif(&[]));
    assert!(!is_avif(&[0; 12]));
    assert!(!is_avif(b"RIFF0000WEBPVP8 "));
}
