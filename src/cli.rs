use clap::{Command, command};

use self::codecs::Codecs;

pub mod codecs;
pub mod common;
pub mod pipeline;
pub mod preprocessors;
pub mod utils;

pub fn cli() -> Command {
    command!()
        .arg_required_else_help(true)
        .after_help(after_help())
        .codecs()
}

/// The help epilogue is generated rather than literal so the codec table
/// only lists what this binary was actually compiled with: several codecs
/// and both preprocessing operations are feature-gated, and a static table
/// would advertise subcommands that do not exist in a trimmed build.
fn after_help() -> String {
    let rows: &[&'static str] = &[
        #[cfg(feature = "avif")]
        "| avif          | O     | O      | Static only       |",
        "| bmp           | O     | X      |                   |",
        "| farbfeld      | O     | O      |                   |",
        "| hdr           | O     | O      |                   |",
        "| jpeg          | O     | O      |                   |",
        "| jpeg_xl(jxl)  | O     | O      |                   |",
        #[cfg(feature = "mozjpeg")]
        "| mozjpeg(moz)  | O     | O      |                   |",
        #[cfg(feature = "oxipng")]
        "| oxipng(oxi)   | O     | O      | Static only       |",
        "| png           | O     | O      | Static only       |",
        "| ppm           | O     | O      |                   |",
        "| psd           | O     | X      |                   |",
        "| qoi           | O     | O      |                   |",
        #[cfg(feature = "svg")]
        "| svg           | O     | X      | Resize losslessly |",
        #[cfg(feature = "tiff")]
        "| tiff          | O     | X      |                   |",
        #[cfg(feature = "webp")]
        "| webp          | O     | O      | Static only       |",
    ];

    let mut text = String::from(
        "\nList of supported codecs\n\
         | Image Format  | Input | Output | Note              |\n\
         | ------------- | ----- | ------ | ----------------- |\n",
    );
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text.push_str("\nList of supported preprocessing options\n");
    #[cfg(feature = "resize")]
    text.push_str("- Resize\n");
    #[cfg(feature = "quantization")]
    text.push_str("- Quantization\n");
    text.push_str("- Alpha premultiply\n");

    text.push_str(
        "\nList of supported mode for output info presenting\n\
         - No-progress (Shown on Default)\n\
         - Quiet (Show all msgs on Default)\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_app() {
        cli().debug_assert();
    }
}
