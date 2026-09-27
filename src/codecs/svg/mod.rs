//! SVG input support rendering vector images through [`resvg`].

pub mod decoder;

pub(crate) mod fonts;

pub use decoder::{SIZE_LIMIT_MARKER, SvgDecoder, SvgOptions, parse_size_limit};
