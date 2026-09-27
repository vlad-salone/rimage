/// AVIF encoding support
#[cfg(feature = "avif")]
pub mod avif;

/// MozJpeg encoding support
#[cfg(feature = "mozjpeg")]
pub mod mozjpeg;

/// OxiPNG encoding support
#[cfg(feature = "oxipng")]
pub mod oxipng;

/// SVG input support
#[cfg(feature = "svg")]
pub mod svg;

/// Tiff encoding support
#[cfg(feature = "tiff")]
pub mod tiff;

/// WebP encoding support
#[cfg(feature = "webp")]
pub mod webp;
