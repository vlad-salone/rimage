use anyhow::anyhow;

/// Markers that select which dimension a resize value is anchored to.
/// Only one of them may appear in a single value.
const ANCHOR_MARKERS: [char; 4] = ['w', 'h', 'l', 's'];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResizeValue {
    Multiplier(f32),
    Percentage(f32),
    Dimensions(Option<usize>, Option<usize>),
    /// Resize so that the longer of the two sides becomes this length.
    LongestSide(usize),
    /// Resize so that the shorter of the two sides becomes this length.
    ShortestSide(usize),
}

impl ResizeValue {
    pub fn map_dimensions(&self, width: usize, height: usize) -> (usize, usize) {
        match self {
            ResizeValue::Multiplier(multiplier) => (
                (width as f32 * multiplier) as usize,
                (height as f32 * multiplier) as usize,
            ),

            ResizeValue::Percentage(percentage) => (
                (width as f32 * (percentage / 100.)) as usize,
                (height as f32 * (percentage / 100.)) as usize,
            ),

            ResizeValue::Dimensions(new_width, new_height) => {
                let aspect_ratio = width as f32 / height as f32;

                let width = new_width.unwrap_or(
                    new_height
                        .map(|h| (h as f32 * aspect_ratio) as usize)
                        .unwrap_or(width),
                );
                let height = new_height.unwrap_or(
                    new_width
                        .map(|w| (w as f32 / aspect_ratio) as usize)
                        .unwrap_or(height),
                );

                (width, height)
            }

            // Which side is the longest depends on the image, so the anchored
            // side is picked per image instead of being fixed by the value.
            ResizeValue::LongestSide(side) => scale_side(width, height, *side, width >= height),
            ResizeValue::ShortestSide(side) => scale_side(width, height, *side, width <= height),
        }
    }
}

/// Scales an image so that one of its sides becomes `side`, preserving the aspect ratio.
///
/// `anchor_is_width` selects the side that is pinned to `side`. The other one is
/// derived from the aspect ratio and never drops below a single pixel, which
/// would otherwise fail the resize on extremely elongated images.
fn scale_side(width: usize, height: usize, side: usize, anchor_is_width: bool) -> (usize, usize) {
    // A degenerate image has no aspect ratio to preserve.
    if width == 0 || height == 0 {
        return (width, height);
    }

    if anchor_is_width {
        let new_height = (side as f32 * height as f32 / width as f32) as usize;

        (side, new_height.max(1))
    } else {
        let new_width = (side as f32 * width as f32 / height as f32) as usize;

        (new_width.max(1), side)
    }
}

/// Extracts the target length from a value anchored to one of the resize markers.
///
/// The marker may sit on either end of the digits, but nothing else may appear
/// in the value. Pulling the first run of digits out of anything else would
/// silently accept typos like `1.5w`, which would resize down to a single pixel.
///
/// A zero length is rejected here rather than at resize time, because it can
/// never produce a valid image and the error is clearer next to the input.
fn parse_length(s: &str, marker: char) -> Result<usize, anyhow::Error> {
    let Some(digits) = s.strip_prefix(marker).or_else(|| s.strip_suffix(marker)) else {
        return Err(anyhow!("Invalid resize value"));
    };

    let length: usize = digits
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid resize value"))?;

    if length == 0 {
        return Err(anyhow!("Length should be greater than 0"));
    }

    Ok(length)
}

/// Parses a positive, finite scale factor for the `@` and `%` forms.
///
/// `f32` parsing accepts strings like `nan` and `inf`, and a zero or negative
/// factor maps every image onto a degenerate target: NaN and negatives become
/// 0 in the float-to-`usize` cast, and infinity saturates to `usize::MAX`.
/// Those failures only surface inside the resize operation, far from the
/// input that caused them, so they are rejected here instead.
fn parse_scale(s: &str) -> Result<f32, anyhow::Error> {
    let scale: f32 = s
        .trim()
        .parse()
        .map_err(|_| anyhow!("Invalid resize value"))?;

    if !scale.is_finite() || scale <= 0.0 {
        return Err(anyhow!(
            "Resize scale factor should be a positive, finite number"
        ));
    }

    Ok(scale)
}

impl std::fmt::Display for ResizeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResizeValue::Multiplier(multiplier) => f.write_fmt(format_args!("@{multiplier}")),
            ResizeValue::Percentage(percentage) => f.write_fmt(format_args!("{percentage}%")),
            ResizeValue::Dimensions(Some(width), Some(height)) => {
                f.write_fmt(format_args!("{width}x{height}"))
            }
            ResizeValue::Dimensions(Some(width), None) => f.write_fmt(format_args!("{width}w")),
            ResizeValue::Dimensions(None, Some(height)) => f.write_fmt(format_args!("{height}h")),
            ResizeValue::Dimensions(None, None) => f.write_fmt(format_args!("base")),
            ResizeValue::LongestSide(side) => f.write_fmt(format_args!("{side}l")),
            ResizeValue::ShortestSide(side) => f.write_fmt(format_args!("{side}s")),
        }
    }
}

impl std::str::FromStr for ResizeValue {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();

        match s {
            s if s.starts_with('@') => Ok(Self::Multiplier(parse_scale(&s[1..])?)),
            s if s.ends_with('%') => Ok(Self::Percentage(parse_scale(&s[..s.len() - 1])?)),
            s if ANCHOR_MARKERS
                .iter()
                .filter(|marker| s.contains(**marker))
                .count()
                > 1 =>
            {
                Err(anyhow!(
                    "Resize value can be anchored to only one of width, height, longest or shortest side"
                ))
            }
            s if s.contains('w') => Ok(Self::Dimensions(Some(parse_length(&s, 'w')?), None)),
            s if s.contains('h') => Ok(Self::Dimensions(None, Some(parse_length(&s, 'h')?))),
            s if s.contains('l') => Ok(Self::LongestSide(parse_length(&s, 'l')?)),
            s if s.contains('s') => Ok(Self::ShortestSide(parse_length(&s, 's')?)),
            s if s.contains('x') => {
                let dimensions: Vec<&str> = s.split('x').collect();
                if dimensions.len() > 2 {
                    return Err(anyhow!("There is more than 2 dimensions"));
                }

                // An empty dimension half (e.g. "x100" or "100x") produces a
                // parse error with an unhelpful message. Surface a clear
                // error naming the missing value.
                let width = Some(dimensions[0].parse::<usize>().map_err(|_| {
                    anyhow!(
                        "Invalid resize width '{}': expected a positive integer",
                        dimensions[0]
                    )
                })?);

                let height = Some(dimensions[1].parse::<usize>().map_err(|_| {
                    anyhow!(
                        "Invalid resize height '{}': expected a positive integer",
                        dimensions[1]
                    )
                })?);

                // `parse_length` rejects a zero length for the anchored forms
                // because it can never produce a valid image; the WxH form is
                // held to the same rule.
                if width == Some(0) || height == Some(0) {
                    return Err(anyhow!("Resize dimensions should be greater than 0"));
                }

                Ok(Self::Dimensions(width, height))
            }
            _ => Err(anyhow!("Invalid resize value")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display() {
        assert_eq!(ResizeValue::Multiplier(2.).to_string(), "@2");
        assert_eq!(ResizeValue::Multiplier(1.5).to_string(), "@1.5");

        assert_eq!(ResizeValue::Percentage(200.).to_string(), "200%");
        assert_eq!(ResizeValue::Percentage(150.).to_string(), "150%");

        assert_eq!(
            ResizeValue::Dimensions(Some(200), Some(200)).to_string(),
            "200x200"
        );
        assert_eq!(
            ResizeValue::Dimensions(Some(150), Some(150)).to_string(),
            "150x150"
        );

        assert_eq!(ResizeValue::Dimensions(None, Some(200)).to_string(), "200h");
        assert_eq!(ResizeValue::Dimensions(None, Some(150)).to_string(), "150h");

        assert_eq!(ResizeValue::Dimensions(Some(200), None).to_string(), "200w");
        assert_eq!(ResizeValue::Dimensions(Some(150), None).to_string(), "150w");

        assert_eq!(ResizeValue::Dimensions(None, None).to_string(), "base");

        assert_eq!(ResizeValue::LongestSide(200).to_string(), "200l");
        assert_eq!(ResizeValue::LongestSide(150).to_string(), "150l");

        assert_eq!(ResizeValue::ShortestSide(200).to_string(), "200s");
        assert_eq!(ResizeValue::ShortestSide(150).to_string(), "150s");
    }

    #[test]
    fn from_str() {
        assert_eq!(
            "@2".parse::<ResizeValue>().unwrap(),
            ResizeValue::Multiplier(2.)
        );

        assert_eq!(
            "@1.5".parse::<ResizeValue>().unwrap(),
            ResizeValue::Multiplier(1.5)
        );

        assert_eq!(
            "200%".parse::<ResizeValue>().unwrap(),
            ResizeValue::Percentage(200.)
        );

        assert_eq!(
            "150%".parse::<ResizeValue>().unwrap(),
            ResizeValue::Percentage(150.)
        );

        assert_eq!(
            "200x200".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(200), Some(200))
        );

        assert_eq!(
            "150x150".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(150), Some(150))
        );

        assert_eq!(
            "200w".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(200), None)
        );

        assert_eq!(
            "150w".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(150), None)
        );

        assert_eq!(
            "200h".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(None, Some(200))
        );

        assert_eq!(
            "150h".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(None, Some(150))
        );

        assert_eq!(
            "200 w".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(200), None)
        );

        assert_eq!(
            "150 w".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(150), None)
        );

        assert_eq!(
            "h200".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(None, Some(200))
        );

        assert_eq!(
            "h150".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(None, Some(150))
        );

        assert_eq!(
            "200l".parse::<ResizeValue>().unwrap(),
            ResizeValue::LongestSide(200)
        );

        assert_eq!(
            "150l".parse::<ResizeValue>().unwrap(),
            ResizeValue::LongestSide(150)
        );

        assert_eq!(
            "200s".parse::<ResizeValue>().unwrap(),
            ResizeValue::ShortestSide(200)
        );

        assert_eq!(
            "150s".parse::<ResizeValue>().unwrap(),
            ResizeValue::ShortestSide(150)
        );

        assert!("_x_".parse::<ResizeValue>().is_err());
        assert!("150wh".parse::<ResizeValue>().is_err());
    }

    #[test]
    fn from_str_side_accepts_same_shapes_as_width_and_height() {
        // Whatever spelling works for `100w` should work for the side values too.
        for value in ["200l", "200 l", "l200", "200L", "200 L", "L200"] {
            assert_eq!(
                value.parse::<ResizeValue>().unwrap(),
                ResizeValue::LongestSide(200),
                "failed to parse {value}"
            );
        }

        for value in ["200s", "200 s", "s200", "200S", "200 S", "S200"] {
            assert_eq!(
                value.parse::<ResizeValue>().unwrap(),
                ResizeValue::ShortestSide(200),
                "failed to parse {value}"
            );
        }
    }

    #[test]
    fn from_str_width_height_accept_marker_on_either_end() {
        for value in ["200w", "200 w", "w200", "200W", "200 W", "W200"] {
            assert_eq!(
                value.parse::<ResizeValue>().unwrap(),
                ResizeValue::Dimensions(Some(200), None),
                "failed to parse {value}"
            );
        }

        for value in ["200h", "200 h", "h200", "200H", "200 H", "H200"] {
            assert_eq!(
                value.parse::<ResizeValue>().unwrap(),
                ResizeValue::Dimensions(None, Some(200)),
                "failed to parse {value}"
            );
        }
    }

    #[test]
    fn from_str_rejects_zero_length() {
        assert!("0w".parse::<ResizeValue>().is_err());
        assert!("0h".parse::<ResizeValue>().is_err());
        assert!("0l".parse::<ResizeValue>().is_err());
        assert!("0s".parse::<ResizeValue>().is_err());
    }

    #[test]
    fn from_str_rejects_zero_dimensions_in_wxh_form() {
        // The anchored forms reject a zero length; the WxH form follows the
        // same rule instead of failing later inside the resize operation.
        for value in ["0x100", "100x0", "0x0"] {
            assert!(
                value.parse::<ResizeValue>().is_err(),
                "{value} should be rejected"
            );
        }
    }

    #[test]
    fn from_str_rejects_non_positive_or_non_finite_scale_factors() {
        // NaN and negative factors cast to a zero size, infinity saturates to
        // usize::MAX; all of them must fail at parse time, not mid-pipeline.
        for value in [
            "@0", "@-2", "@nan", "@inf", "@-inf", "0%", "-5%", "nan%", "inf%",
        ] {
            assert!(
                value.parse::<ResizeValue>().is_err(),
                "{value} should be rejected"
            );
        }
    }

    #[test]
    fn from_str_rejects_anything_around_the_digits() {
        // Taking the first run of digits out of the value would turn `1.5w`
        // into a single pixel image instead of an error.
        for value in [
            "1.5l", "1.5s", "200l300", "200s300", "abc200l", "200sfoo", "2 0 0 l", "200.l",
            "200lol", "l200l", "1.5w", "1.5h", "abc100w", "100w200", "w=100", "100h50", "100h200",
            "h=100", "100w100", "2 0 0 w", "200.h", "w200w", "200ww",
        ] {
            assert!(
                value.parse::<ResizeValue>().is_err(),
                "{value} should not parse"
            );
        }
    }

    #[test]
    fn from_str_rejects_missing_length() {
        assert!("w".parse::<ResizeValue>().is_err());
        assert!("h".parse::<ResizeValue>().is_err());
        assert!("l".parse::<ResizeValue>().is_err());
        assert!("s".parse::<ResizeValue>().is_err());
    }

    #[test]
    fn from_str_rejects_multiple_anchors() {
        for value in [
            "150ls", "150wl", "150ws", "150hl", "150hs", "150wh", "150lw", "150sh",
        ] {
            assert!(
                value.parse::<ResizeValue>().is_err(),
                "{value} should not parse"
            );
        }
    }

    #[test]
    fn from_str_still_accepts_values_without_anchors() {
        // The anchor check runs before every dimension branch, so make sure it
        // does not reject the values that carry no anchor marker at all.
        assert_eq!(
            "100x100".parse::<ResizeValue>().unwrap(),
            ResizeValue::Dimensions(Some(100), Some(100))
        );

        assert_eq!(
            "@2".parse::<ResizeValue>().unwrap(),
            ResizeValue::Multiplier(2.)
        );

        assert_eq!(
            "150%".parse::<ResizeValue>().unwrap(),
            ResizeValue::Percentage(150.)
        );
    }

    #[test]
    fn display_round_trips_through_from_str() {
        for value in [
            ResizeValue::Multiplier(1.5),
            ResizeValue::Percentage(150.),
            ResizeValue::Dimensions(Some(100), Some(200)),
            ResizeValue::Dimensions(Some(100), None),
            ResizeValue::Dimensions(None, Some(200)),
            ResizeValue::LongestSide(1000),
            ResizeValue::ShortestSide(1000),
        ] {
            assert_eq!(
                value.to_string().parse::<ResizeValue>().unwrap(),
                value,
                "{value} did not round trip"
            );
        }
    }

    #[test]
    fn map_dimensions_longest_side() {
        let resize_value = ResizeValue::LongestSide(100);

        // Landscape anchors on the width, portrait on the height, and the
        // result has the same longest side either way.
        assert_eq!(resize_value.map_dimensions(1000, 500), (100, 50));
        assert_eq!(resize_value.map_dimensions(500, 1000), (50, 100));
        assert_eq!(resize_value.map_dimensions(800, 800), (100, 100));
    }

    #[test]
    fn map_dimensions_shortest_side() {
        let resize_value = ResizeValue::ShortestSide(100);

        assert_eq!(resize_value.map_dimensions(1000, 500), (200, 100));
        assert_eq!(resize_value.map_dimensions(500, 1000), (100, 200));
        assert_eq!(resize_value.map_dimensions(800, 800), (100, 100));
    }

    #[test]
    fn map_dimensions_side_upscales() {
        assert_eq!(
            ResizeValue::LongestSide(1000).map_dimensions(100, 50),
            (1000, 500)
        );

        assert_eq!(
            ResizeValue::ShortestSide(1000).map_dimensions(100, 50),
            (2000, 1000)
        );
    }

    #[test]
    fn map_dimensions_side_is_a_noop_when_the_image_already_fits() {
        // The pipeline skips the resize when the mapped size equals the source,
        // so this is what makes an already correct image untouched.
        assert_eq!(
            ResizeValue::LongestSide(1000).map_dimensions(1000, 500),
            (1000, 500)
        );

        assert_eq!(
            ResizeValue::ShortestSide(500).map_dimensions(1000, 500),
            (1000, 500)
        );

        assert_eq!(
            ResizeValue::LongestSide(800).map_dimensions(800, 800),
            (800, 800)
        );
    }

    #[test]
    fn map_dimensions_side_never_returns_zero() {
        // Extreme aspect ratios truncate the derived side to zero, which the
        // resize operation rejects outright.
        assert_eq!(
            ResizeValue::LongestSide(10).map_dimensions(1000, 3),
            (10, 1)
        );
        assert_eq!(
            ResizeValue::LongestSide(10).map_dimensions(3, 1000),
            (1, 10)
        );
        assert_eq!(
            ResizeValue::LongestSide(1).map_dimensions(1000, 1000),
            (1, 1)
        );
    }

    #[test]
    fn map_dimensions_side_preserves_aspect_ratio() {
        for (width, height) in [(1920, 1080), (1080, 1920), (640, 640), (3000, 2000)] {
            for side in [50, 100, 512, 4000] {
                for value in [
                    ResizeValue::LongestSide(side),
                    ResizeValue::ShortestSide(side),
                ] {
                    let (new_width, new_height) = value.map_dimensions(width, height);

                    let source_ratio = width as f32 / height as f32;
                    let result_ratio = new_width as f32 / new_height as f32;

                    // Truncation moves the derived side by at most a pixel, so
                    // compare with a tolerance scaled to the smaller result.
                    let tolerance = source_ratio / new_width.min(new_height) as f32;

                    assert!(
                        (source_ratio - result_ratio).abs() <= tolerance,
                        "{value} on {width}x{height} gave {new_width}x{new_height}"
                    );
                }
            }
        }
    }

    #[test]
    fn map_dimensions_side_anchors_the_expected_side() {
        for (width, height) in [(1920, 1080), (1080, 1920), (640, 640), (3000, 2000)] {
            let (new_width, new_height) =
                ResizeValue::LongestSide(512).map_dimensions(width, height);
            assert_eq!(new_width.max(new_height), 512);

            let (new_width, new_height) =
                ResizeValue::ShortestSide(512).map_dimensions(width, height);
            assert_eq!(new_width.min(new_height), 512);
        }
    }

    #[test]
    fn map_dimensions_side_leaves_degenerate_images_alone() {
        assert_eq!(
            ResizeValue::LongestSide(100).map_dimensions(0, 500),
            (0, 500)
        );
        assert_eq!(
            ResizeValue::ShortestSide(100).map_dimensions(500, 0),
            (500, 0)
        );
        assert_eq!(ResizeValue::LongestSide(100).map_dimensions(0, 0), (0, 0));
    }

    #[test]
    fn map_dimensions_multiplier() {
        let resize_value = ResizeValue::Multiplier(2.0);
        assert_eq!(resize_value.map_dimensions(100, 200), (200, 400));
    }

    #[test]
    fn map_dimensions_percentage() {
        let resize_value = ResizeValue::Percentage(50.0);
        assert_eq!(resize_value.map_dimensions(100, 200), (50, 100));
    }

    #[test]
    fn map_dimensions_dimensions() {
        let resize_value = ResizeValue::Dimensions(Some(300), Some(600));
        assert_eq!(resize_value.map_dimensions(100, 200), (300, 600));
    }

    #[test]
    fn map_dimensions_dimensions_with_width() {
        let resize_value = ResizeValue::Dimensions(Some(300), None);
        assert_eq!(resize_value.map_dimensions(100, 200), (300, 600));
    }

    #[test]
    fn map_dimensions_dimensions_with_height() {
        let resize_value = ResizeValue::Dimensions(None, Some(600));
        assert_eq!(resize_value.map_dimensions(100, 200), (300, 600));
    }

    #[test]
    fn map_dimensions_dimensions_with_none() {
        let resize_value = ResizeValue::Dimensions(None, None);
        assert_eq!(resize_value.map_dimensions(100, 200), (100, 200));
    }
}
