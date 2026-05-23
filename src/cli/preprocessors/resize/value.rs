use std::sync::LazyLock;

use anyhow::anyhow;
use regex::Regex;

static WIDTH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?P<width>\d+)").unwrap());
static HEIGHT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?P<height>\d+)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResizeValue {
    Multiplier(f32),
    Percentage(f32),
    Dimensions(Option<usize>, Option<usize>),
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
        }
    }
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
        }
    }
}

impl std::str::FromStr for ResizeValue {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();

        match s {
            s if s.starts_with('@') => Ok(Self::Multiplier(s[1..].parse()?)),
            s if s.ends_with('%') => Ok(Self::Percentage(s[..s.len() - 1].parse()?)),
            s if s.contains('w') && s.contains('h') => Err(anyhow!("Invalid resize value")),
            s if s.contains('w') => {
                let Some(cap) = WIDTH_RE.captures(&s) else {
                    return Err(anyhow!("Invalid resize value"));
                };

                Ok(Self::Dimensions(Some(cap["width"].parse()?), None))
            }
            s if s.contains('h') => {
                let Some(cap) = HEIGHT_RE.captures(&s) else {
                    return Err(anyhow!("Invalid resize value"));
                };

                Ok(Self::Dimensions(None, Some(cap["height"].parse()?)))
            }
            s if s.contains('x') => {
                let dimensions: Vec<&str> = s.split('x').collect();
                if dimensions.len() > 2 {
                    return Err(anyhow!("There is more that 2 dimensions"));
                }

                let width = Some(dimensions[0].parse::<usize>()?);

                let height = Some(dimensions[1].parse::<usize>()?);

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

        assert!("_x_".parse::<ResizeValue>().is_err());
        assert!("150wh".parse::<ResizeValue>().is_err());
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
