use std::fs::File;

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
fn high_depth_ycgco_round_trip() {
    // forward YCgCo (H.273 matrix 8) of R=200 G=50 B=100 in the 8-bit domain:
    // Y=100, Cg=-50, Co=+50, scaled into full-range 10-bit samples
    let y = u16_plane(&[400; 32], 16, 4);
    let u = u16_plane(&[412; 8], 8, 2);
    let v = u16_plane(&[612; 8], 8, 2);
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
