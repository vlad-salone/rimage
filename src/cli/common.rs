use std::path::PathBuf;

use clap::{ArgAction, Command, arg, value_parser};
use indoc::indoc;

use super::preprocessors::Preprocessors;

impl CommonArgs for Command {
    fn common_args(self) -> Self {
        let cmd = self
        .next_help_heading("General").args([
            arg!(files: [FILES] ... "Input file(s) to process.")
                .long_help(indoc! {r#"Input file(s) to process.

                If the file path contains spaces, enclose the path with double quotation marks on both sides.

                A file named `file.list` is read as a UTF-8 file list with one input file per line. Blank lines are skipped, surrounding whitespace is ignored, and relative paths are resolved against the current working directory. When a `file.list` is provided, all other input file arguments are ignored."#})
                .required_unless_present("print-limits")
                .value_parser(value_parser!(PathBuf)),
            arg!(-d --directory <DIR> "The directory to write output file(s) to.")
                .long_help(indoc! {r#"The directory to write output file(s) to.

                Output files will be written without preserving the folder structure unless the --recursive flag is used."#})
                .value_parser(value_parser!(PathBuf)),
            arg!(-r --recursive "Preserves the folder structure when writing output file(s).")
                .long_help(indoc! {r#"Preserves the folder structure when writing output file(s).

                This option should be used in conjunction with the --directory option."#})
                .requires("directory"),
            arg!(-s --suffix [SUFFIX] "Adds the suffix to the names of output file(s).")
                .long_help(indoc! {r#"Adds the suffix to the names of output file(s).

                When '2x' is provided as the value, the resulting files will be renamed with the '2x' suffix.
                For example, a file named 'file.jpeg' will become 'file2x.jpeg'.

                If no suffix is provided, the default suffix 'updated' will be added to the resulting files."#})
                .default_missing_value("updated"),
            arg!(-b --backup "Adds the '@backup' to the names of input file(s)."),
            arg!(-t --threads <NUM> "The maximum number of images to process concurrently.")
                .long_help(indoc! {r#"The maximum number of images to process concurrently.

                Limits how many images are decoded and held in memory at once.
                Higher values increase speed but use more RAM, which may cause out-of-memory errors with large images.
                By default, processes one image at a time (--threads 1).

                A value larger than this machine's parallelism is reduced to it,
                rather than rejected: the same command line then works on
                machines with different core counts."#})
                // u16, not u8: a u8 would cap a large machine at 255 workers.
                //
                // The parser accepts anything positive and `utils::threads`
                // clamps it to the machine's parallelism, so an over-large
                // value is a warning rather than a usage error. Bounding the
                // range here instead would make a fixed `-t 16` script fail
                // outright on any machine with fewer cores.
                .value_parser(value_parser!(u16).range(1i64..=u16::MAX as i64)),
            arg!(-x --strip "Strip metadata when encoding images (where supported)")
                .action(ArgAction::SetTrue),
            arg!(--"no-progress" "Disables progress bar.")
                .long_help(indoc! {r#"Disables progress bar.

                By default, progress bar is enabled."#}),
            arg!(--quiet "Disables all output.")
                .long_help(indoc! {r#"Disables all output.

                By default, all output is enabled."#}),
            #[cfg(feature = "metadata")]
            arg!(--metadata [FILE] "Outputs metadata of the processed image(s).")
                .long_help(indoc! {r#"Outputs metadata of the processed image(s).

                This will output the metadata of the processed image(s) in JSON format."#})
                .value_parser(value_parser!(PathBuf)),
            arg!(--"print-limits" "Print the runtime-derived size limits and exit.")
                .long_help(indoc! {r#"Print the runtime-derived size limits and exit.

                Shows the probed system memory budget, the per-format dimension and pixel
                ceilings, and which source (format, memory, or disk) is the binding
                constraint. No files are processed. This is a diagnostic tool for
                understanding why an image was rejected or for calibrating the
                pipeline cost estimates."#})
                .action(ArgAction::SetTrue)
                .hide(true),
        ]);

        cmd.preprocessors()
    }
}

pub trait CommonArgs {
    fn common_args(self) -> Self;
}
