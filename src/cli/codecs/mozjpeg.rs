use clap::{Command, arg, value_parser};

use crate::cli::common::CommonArgs;

pub fn mozjpeg() -> Command {
    Command::new("mozjpeg")
        .alias("moz")
        .about("Encode images into JPEG format using MozJpeg codec. (RECOMMENDED and Small)")
        .args([
            arg!(-q --quality <NUM> "Quality, values 60-80 are recommended.")
                .value_parser(value_parser!(u8).range(1..=100))
                .default_value("75"),
            arg!(--chroma_quality <NUM> "Separate chroma quality.")
                .value_parser(value_parser!(u8).range(1..=100)),
            arg!(--baseline "Set to use baseline encoding (by default is progressive)."),
            arg!(--no_optimize_coding "Set to make files larger for no reason."),
            arg!(--smoothing <NUM> "Use MozJPEG's smoothing.")
                .value_parser(value_parser!(u8).range(1..=100)),
            arg!(--colorspace <COLOR> "Set color space of JPEG being written.")
                .value_parser(["ycbcr", "grayscale", "rgb"])
                .default_value("ycbcr"),
            arg!(--multipass "Specifies whether multiple scans should be considered during trellis quantization."),
            arg!(--subsample <PIX> "Sets chroma subsampling.")
                .long_help(
                    "Sets chroma subsampling for the output JPEG.\n\
                     \n\
                     1 = 4:4:4 (no chroma subsampling, best color fidelity), \
                     2 = 4:2:0 (default at most quality levels, smallest file), \
                     3 and 4 = NxN sampling factors that downsample chroma even \
                     further; legal but nonstandard, and rarely useful outside \
                     file-size experiments.\n\
                     \n\
                     By default rimage lets MozJPEG pick automatically; at \
                     typical qualities it picks 2 (4:2:0), which throws away \
                     three quarters of the chroma resolution. Saturated colors \
                     (e.g. bright red) can become visibly darker or duller. \
                     Use --subsample 1 to preserve color detail at the cost of \
                     a larger file.")
                .value_parser(value_parser!(u8).range(1..=4)),
            arg!(--qtable <TABLE> "Use a specific quantization table.")
                .long_help(
                    "Use a specific quantization table.\n\
                     \n\
                     rimage defaults to NRobidoux, which favors smaller files. \
                     Other tools such as ImageMagick typically use the standard \
                     Annex K tables; together with chroma subsampling this can \
                     produce small color differences at the same quality value.")
                .value_parser([
                    "AhumadaWatsonPeterson",
                    "AnnexK",
                    "Flat",
                    "KleinSilversteinCarney",
                    "MSSSIM",
                    "NRobidoux",
                    "PSNRHVS",
                    "PetersonAhumadaWatson",
                    "WatsonTaylorBorthwick"
                ])
                .default_value("NRobidoux")
        ]).common_args()
}
