use std::io::Write;
use std::path::PathBuf;

use super::{JfifDensity, insert_jpeg_exif_app1, read_jpeg_source_metadata};

const SOI: [u8; 2] = [0xFF, 0xD8];
const EOI: [u8; 2] = [0xFF, 0xD9];

/// Writes `bytes` to a scratch file and returns its path.
///
/// The scanner takes a path rather than a reader, so these tests build real
/// files. Each test passes a distinct name so they cannot collide.
fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rimage-jpeg-test-{name}.jpg"));
    let mut file = std::fs::File::create(&path).expect("create the scratch file");
    file.write_all(bytes).expect("write the scratch file");
    path
}

/// Builds a metadata segment: `FF <marker> <length> <payload>`, where `length`
/// counts the two length bytes themselves.
fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let length = (payload.len() + 2) as u16;
    let mut out = vec![0xFF, marker, (length >> 8) as u8, (length & 0xFF) as u8];
    out.extend_from_slice(payload);
    out
}

/// Builds an APP0/JFIF payload. The scanner reads `unit` at byte 7 and the two
/// densities as big-endian at bytes 8..12.
fn jfif(unit: u8, x_density: u16, y_density: u16) -> Vec<u8> {
    let mut out = b"JFIF\0".to_vec();
    out.extend_from_slice(&[1, 2]); // version 1.2
    out.push(unit);
    out.extend_from_slice(&x_density.to_be_bytes());
    out.extend_from_slice(&y_density.to_be_bytes());
    out.extend_from_slice(&[0, 0]); // no thumbnail
    out
}

/// Builds an APP1/EXIF payload whose body identifies it to the assertions.
fn exif(body: &[u8]) -> Vec<u8> {
    let mut out = b"Exif\0\0".to_vec();
    out.extend_from_slice(body);
    out
}

/// Wraps metadata segments in SOI/EOI so the scanner has a well-formed file.
fn jpeg(segments: &[Vec<u8>]) -> Vec<u8> {
    let mut out = SOI.to_vec();
    for segment in segments {
        out.extend_from_slice(segment);
    }
    out.extend_from_slice(&EOI);
    out
}

#[test]
fn jfif_density_is_parsed_from_the_app0_segment() {
    let path = scratch("density", &jpeg(&[segment(0xE0, &jfif(1, 300, 300))]));

    let metadata = read_jpeg_source_metadata(&path)
        .unwrap()
        .expect("is a jpeg");

    assert_eq!(
        metadata.jfif_density,
        Some(JfifDensity {
            unit: 1,
            x_density: 300,
            y_density: 300,
        })
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn the_first_exif_segment_is_preserved() {
    let expected = exif(b"first");
    let path = scratch("exif-first", &jpeg(&[segment(0xE1, &expected)]));

    let metadata = read_jpeg_source_metadata(&path)
        .unwrap()
        .expect("is a jpeg");

    assert_eq!(metadata.exif_app1.as_deref(), Some(expected.as_slice()));
    std::fs::remove_file(path).ok();
}

/// The guard that keeps the first copy of a segment is what sends a repeat to
/// the skip arm. Losing it would let a later segment overwrite the earlier
/// one, which is what these two tests exist to catch.
#[test]
fn a_second_exif_segment_does_not_replace_the_first() {
    let expected = exif(b"first");
    let path = scratch(
        "exif-repeat",
        &jpeg(&[segment(0xE1, &expected), segment(0xE1, &exif(b"second"))]),
    );

    let metadata = read_jpeg_source_metadata(&path)
        .unwrap()
        .expect("is a jpeg");

    assert_eq!(
        metadata.exif_app1.as_deref(),
        Some(expected.as_slice()),
        "a repeated APP1 must be skipped, not overwrite the one already held"
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn a_second_jfif_segment_does_not_replace_the_first() {
    let path = scratch(
        "jfif-repeat",
        &jpeg(&[
            segment(0xE0, &jfif(1, 300, 300)),
            segment(0xE0, &jfif(2, 118, 118)),
        ]),
    );

    let metadata = read_jpeg_source_metadata(&path)
        .unwrap()
        .expect("is a jpeg");

    assert_eq!(
        metadata.jfif_density,
        Some(JfifDensity {
            unit: 1,
            x_density: 300,
            y_density: 300,
        }),
        "a repeated APP0 must be skipped, not overwrite the density already held"
    );
    std::fs::remove_file(path).ok();
}

/// XMP lives in APP1 as well, so an APP1 marker alone does not make a segment
/// EXIF. A non-EXIF APP1 must not consume the one EXIF slot.
#[test]
fn an_app1_that_is_not_exif_does_not_claim_the_exif_slot() {
    let expected = exif(b"real");
    let path = scratch(
        "exif-after-xmp",
        &jpeg(&[
            segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>"),
            segment(0xE1, &expected),
        ]),
    );

    let metadata = read_jpeg_source_metadata(&path)
        .unwrap()
        .expect("is a jpeg");

    assert_eq!(metadata.exif_app1.as_deref(), Some(expected.as_slice()));
    std::fs::remove_file(path).ok();
}

#[test]
fn a_file_without_the_jpeg_signature_is_reported_as_none() {
    let path = scratch("not-a-jpeg", b"definitely not a jpeg");

    assert!(read_jpeg_source_metadata(&path).unwrap().is_none());
    std::fs::remove_file(path).ok();
}

#[test]
fn a_jpeg_without_metadata_segments_returns_empty_metadata() {
    let path = scratch("bare", &jpeg(&[]));

    let metadata = read_jpeg_source_metadata(&path)
        .unwrap()
        .expect("is a jpeg");

    assert!(metadata.jfif_density.is_none());
    assert!(metadata.exif_app1.is_none());
    std::fs::remove_file(path).ok();
}

/// The segment length field covers its own two bytes, so a payload above
/// 65533 bytes would overflow the u16 length — a panic in debug builds and a
/// wrapped, malformed segment in release. It must be rejected, leaving the
/// file untouched.
#[test]
fn an_oversized_exif_payload_is_rejected_without_touching_the_file() {
    let original = jpeg(&[]);
    let path = scratch("exif-oversized", &original);
    let payload = vec![0u8; 65534];

    let result = insert_jpeg_exif_app1(&path, &payload);

    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_file(path).ok();
}

/// The largest legal payload is 65533 bytes, making the declared segment
/// length exactly u16::MAX.
#[test]
fn the_largest_legal_exif_payload_is_written() {
    let path = scratch("exif-max", &jpeg(&[]));
    let payload = vec![0u8; 65533];

    insert_jpeg_exif_app1(&path, &payload).unwrap();

    let data = std::fs::read(&path).unwrap();
    assert_eq!(&data[2..4], &[0xFF, 0xE1]);
    assert_eq!(u16::from_be_bytes([data[4], data[5]]), u16::MAX);
    std::fs::remove_file(path).ok();
}
