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
                .value_parser(value_parser!(u8).range(1..=4)),
            arg!(--qtable <TABLE> "Use a specific quantization table.")
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
