#![allow(unused_imports)]

use clap::{Arg, ArgAction, ArgGroup, Command, arg, value_parser};
use indoc::indoc;

#[cfg(feature = "resize")]
pub use resize::{ResizeFilter, ResizeValue};

#[cfg(feature = "resize")]
mod resize;

impl Preprocessors for Command {
    #[cfg(any(feature = "resize", feature = "quantization"))]
    fn preprocessors(self) -> Self {
        self.group(
                ArgGroup::new("preprocessors")
                    .args([
                        #[cfg(feature = "resize")]
                        "resize",
                        #[cfg(feature = "quantization")]
                         "quantization",
                    ])
                    .multiple(true)
            )
            .next_help_heading("Preprocessors")
            .args([
                #[cfg(feature = "resize")]
                arg!(--resize <RESIZE> "Resize the image(s) according to the specified criteria.")
                    .long_help(indoc! {r#"Resize the image(s) according to the specified criteria.

                    Possible values:
                    - @1.5:    Enlarge image size by this multiplier
                    - 150%:    Adjust image size by this percentage
                    - 100x100: Resize image to Width×Height
                    - 100w:    Adjust image dimensions while maintaining the aspect ratio based on the width
                    - 100h:    Adjust image dimensions while maintaining the aspect ratio based on the height"#})
                    .value_parser(value_parser!(ResizeValue))
                    .action(ArgAction::Append),

                #[cfg(feature = "resize")]
                arg!(--downscale "Downscale the image(s) when resizing.")
                    .long_help(indoc! {r#"Downscale the image(s) when resizing.

                    This is useful when you want to reduce the size of the image when it is larger than the specified size.
                    It is recommended to use this option with --resize"#})
                    .default_value("true")
                    .action(ArgAction::SetTrue)
                    .requires("resize")
                    .overrides_with("no-downscale"),
                #[cfg(feature = "resize")]
                arg!(--"no-downscale" "Disable downscaling when resizing.")
                    .long_help(indoc! {r#"Disable downscaling when resizing.

                    This is useful when you don't want to reduce the size of the image when it is larger than the specified size.
                    It is recommended to use this option with --resize"#})
                    .action(ArgAction::SetTrue)
                    .requires("resize"),

                #[cfg(feature = "resize")]
                arg!(--upscale "Upscale the image(s) when resizing.")
                    .long_help(indoc! {r#"Upscale the image(s) when resizing.

                    This is useful when you want to increase the size of the image when it is smaller than the specified size.
                    It is recommended to use this option with --resize"#})
                    .default_value("true")
                    .action(ArgAction::SetTrue)
                    .requires("resize")
                    .overrides_with("no-upscale"),
                #[cfg(feature = "resize")]
                arg!(--"no-upscale" "Disable upscaling when resizing.")
                    .long_help(indoc! {r#"Disable upscaling when resizing.

                    This is useful when you don't want to increase the size of the image when it is smaller than the specified size.
                    It is recommended to use this option with --resize"#})
                    .action(ArgAction::SetTrue)
                    .requires("resize"),

                #[cfg(feature = "resize")]
                arg!(--filter <FILTER> "Filter that used when resizing an image.")
                    .value_parser(value_parser!(ResizeFilter))
                    .default_value("lanczos3")
                    .requires("resize"),

                #[cfg(feature = "quantization")]
                arg!(--quantization [QUALITY] "Reduces the color palette to the given quality percentage.")
                    .long_help(indoc! {r#"Reduces the color palette to the given quality percentage.

                    This is a preprocessing step that limits how many distinct
                    colors the image can use, not a replacement for -q/--quality
                    (which controls encoder compression).

                    Lower values produce fewer colors, which can introduce
                    banding artifacts. For best file-size reduction, combine
                    with a lower -q value, e.g. -q 50 --quantization 80.

                    If quality is not provided, default 75 is used."#})
                    .value_parser(value_parser!(u8).range(1..=100))
                    .action(ArgAction::Append)
                    .default_missing_value("75"),

                #[cfg(feature = "quantization")]
                arg!(--dithering [QUALITY] "Smooths banding artifacts introduced by quantization.")
                    .long_help(indoc! {r#"Smooths banding artifacts introduced by quantization.

                    Higher values spread quantization error more evenly,
                    reducing visible banding at the cost of slightly
                    larger file sizes.

                    Used with --quantization flag.
                    If quality is not provided, default 75 is used."#})
                    .value_parser(value_parser!(u8).range(1..=100))
                    .default_missing_value("75")
                    .requires("quantization"),

                position_sensitive_flag(arg!(--premultiply "Premultiply alpha before operation"))
                    .action(ArgAction::Append)
            ])
    }
}

fn position_sensitive_flag(arg: Arg) -> Arg {
    // Flags don't track the position of each occurrence, so we need to emulate flags with
    // value-less options to get the same result
    arg.num_args(0)
        .value_parser(value_parser!(bool))
        .default_missing_value("true")
        .default_value("false")
}

pub trait Preprocessors {
    fn preprocessors(self) -> Self;
}
