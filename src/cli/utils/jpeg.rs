use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

/// JFIF density parsed from a JPEG APP0 segment.
///
/// `unit` uses the same values as the JFIF standard:
/// 0 = aspect ratio only, 1 = dots per inch, 2 = dots per cm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JfifDensity {
    pub unit: u8,
    pub x_density: u16,
    pub y_density: u16,
}

/// Metadata rimage can preserve from a JPEG source file.
#[derive(Debug)]
pub struct JpegSourceMetadata {
    pub jfif_density: Option<JfifDensity>,
    /// Raw payload of the first APP1 segment starting with `Exif\0\0`.
    /// The payload does not include the marker or the segment length.
    pub exif_app1: Option<Vec<u8>>,
}

/// Scans the beginning of a JPEG file for the JFIF APP0 density and the first
/// EXIF APP1 segment.
///
/// Only the marker header is read; image data after the start-of-scan marker
/// is never touched. Returns `Ok(None)` when the file is not a JPEG (does not
/// start with the JPEG SOI marker) and an error when the JPEG segment
/// structure is malformed.
pub fn read_jpeg_source_metadata(path: &Path) -> io::Result<Option<JpegSourceMetadata>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let mut signature = [0u8; 2];
    match reader.read_exact(&mut signature) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    if signature != [0xFF, 0xD8] {
        return Ok(None);
    }

    let mut metadata = JpegSourceMetadata {
        jfif_density: None,
        exif_app1: None,
    };

    loop {
        let marker = read_marker(&mut reader)?;

        // End of image; nothing after this point matters.
        if marker == 0xD9 {
            break;
        }

        // Standalone markers (including restart markers) carry no length.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        // A SOI marker here would be malformed, but skipping it keeps the
        // scanner permissive in the same way the rest of the decoder is.
        if marker == 0xD8 {
            continue;
        }

        let segment_len = read_segment_len(&mut reader)?;
        if segment_len < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed JPEG segment length",
            ));
        }
        let remaining = segment_len - 2;

        // The `is_none()` guards keep the first copy of each segment. A second
        // one falls through to `_`, which skips it without reading.
        match marker {
            // APP0: JFIF density lives here.
            0xE0 if metadata.jfif_density.is_none() => {
                let mut prefix = [0u8; 14];
                let read = read_up_to(&mut reader, &mut prefix, remaining)?;
                if read >= 14 && prefix.starts_with(b"JFIF\0") {
                    metadata.jfif_density = Some(JfifDensity {
                        unit: prefix[7],
                        x_density: u16::from_be_bytes([prefix[8], prefix[9]]),
                        y_density: u16::from_be_bytes([prefix[10], prefix[11]]),
                    });
                }
                skip_remaining(&mut reader, remaining, read)?;
            }
            // APP1: the first EXIF segment is the one to preserve.
            0xE1 if metadata.exif_app1.is_none() => {
                let mut prefix = [0u8; 6];
                let read = read_up_to(&mut reader, &mut prefix, remaining)?;
                if read >= 6 && prefix.starts_with(b"Exif\0\0") {
                    let mut payload = Vec::with_capacity(remaining);
                    payload.extend_from_slice(&prefix);
                    payload.resize(remaining, 0);
                    reader.read_exact(&mut payload[6..])?;
                    metadata.exif_app1 = Some(payload);
                } else {
                    skip_remaining(&mut reader, remaining, read)?;
                }
            }
            // Every other segment, and any repeat of one already collected.
            _ => {
                skip_remaining(&mut reader, remaining, 0)?;
            }
        }

        // Metadata segments only appear before the start of scan.
        if marker == 0xDA {
            break;
        }
    }

    Ok(Some(metadata))
}

/// Reads the next marker code after a 0xFF prefix.
fn read_marker(reader: &mut BufReader<File>) -> io::Result<u8> {
    let mut byte = [0u8; 1];

    loop {
        reader.read_exact(&mut byte)?;
        if byte[0] == 0xFF {
            break;
        }
    }

    // Skip any extra 0xFF fill bytes.
    loop {
        reader.read_exact(&mut byte)?;
        if byte[0] != 0xFF {
            return Ok(byte[0]);
        }
    }
}

fn read_segment_len(reader: &mut BufReader<File>) -> io::Result<usize> {
    let mut length = [0u8; 2];
    reader.read_exact(&mut length)?;
    Ok(u16::from_be_bytes(length) as usize)
}

/// Reads up to `buf.len()` bytes, returning how many were actually read before
/// hitting EOF. A short read is not an error here because the caller decides
/// how to handle truncated optional segments.
fn read_up_to(reader: &mut BufReader<File>, buf: &mut [u8], remaining: usize) -> io::Result<usize> {
    let want = buf.len().min(remaining);
    let mut read = 0;
    while read < want {
        match reader.read(&mut buf[read..want]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(read)
}

fn skip_remaining(
    reader: &mut BufReader<File>,
    remaining: usize,
    already_read: usize,
) -> io::Result<()> {
    if remaining > already_read {
        reader.seek(SeekFrom::Current((remaining - already_read) as i64))?;
    }
    Ok(())
}

/// Inserts a raw EXIF APP1 payload into an encoded JPEG file, right after
/// the SOI marker.
///
/// `exif_payload` must be the APP1 payload (starting with `Exif\0\0`) without
/// the marker or length bytes, exactly as returned by
/// [`read_jpeg_source_metadata`].
pub fn insert_jpeg_exif_app1(path: &Path, exif_payload: &[u8]) -> io::Result<()> {
    let data = std::fs::read(path)?;

    if !data.starts_with(&[0xFF, 0xD8]) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "encoded output is not a JPEG",
        ));
    }

    // The segment length field covers its own two bytes, so the largest
    // payload that fits is 65533. Checking against `u16::MAX` directly would
    // let a 65534/65535-byte payload through and overflow `payload_len + 2` —
    // a panic in debug builds and a wrapped, malformed length in release.
    const MAX_APP1_PAYLOAD: usize = u16::MAX as usize - 2;

    if exif_payload.len() > MAX_APP1_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "EXIF APP1 payload is too large for a JPEG segment",
        ));
    }
    let segment_len = (exif_payload.len() + 2) as u16;

    let mut segment = Vec::with_capacity(4 + exif_payload.len());
    segment.extend_from_slice(&[0xFF, 0xE1]);
    segment.extend_from_slice(&segment_len.to_be_bytes());
    segment.extend_from_slice(exif_payload);

    let mut out = Vec::with_capacity(data.len() + segment.len());
    out.extend_from_slice(&data[..2]);
    out.extend_from_slice(&segment);
    out.extend_from_slice(&data[2..]);

    std::fs::write(path, out)
}

#[cfg(test)]
mod tests;
