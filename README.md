# Rimage

[![build status](https://img.shields.io/github/actions/workflow/status/SalOne22/rimage/rimage.yml?label=rimage&style=flat-square)](https://github.com/SalOne22/rimage/actions?query=branch%3Amain+)
[![docs.rs](https://img.shields.io/docsrs/rimage/latest?style=flat-square)](https://docs.rs/rimage)
[![version](https://img.shields.io/crates/v/rimage?style=flat-square)](https://crates.io/crates/rimage)
[![license](https://img.shields.io/crates/l/rimage?style=flat-square)](https://github.com/SalOne22/rimage)

A powerful Rust image optimization CLI tool inspired by [squoosh!](https://squoosh.app/).

## Features

- Modern codecs:
  - Rimage uses modern codecs optimized to produce tiny images
  - Under the hood uses `zune_image` crate that enhances performance
- Optimization operations:
  - Rimage provides several image optimization operation
  - Resize - uses `fast_image_resize` crate that has incredible performance
  - Quantization - allowing to reduce image palette
- Friendly output:
  - Rimage support progressbar
  - Rimage would show detailed error info to assist users
- CJK and Punctuation marks support:
  - Rimage supports full CJK (Chinese, Japanese and Korean) characters input and output
  - Rimage allows special punctuation characters such as `|`, ` `, `&`, `$`, etc. to be included in file names

## Installation

You can download latest release from the [releases](https://github.com/SalOne22/rimage/releases) tab.

If you're a Rust programmer, rimage can be installed with `cargo`.

```sh
cargo install rimage
```

Alternatively, one can use [cargo binstall](https://github.com/cargo-bins/cargo-binstall) to install a rimage binary directly from GitHub:

```sh
cargo binstall rimage
```

> ### Note
>
> If you're a user who just want to **use Rimage easily with a friendly GUI**, [Rimage_gui](https://github.com/Mikachu2333/rimage_gui/releases/) may be fit for you, it support both Chinese and English. Just select the version you need and download it to use.

## Usage

```text
Usage: rimage.exe [COMMAND]

Commands:
  avif      Encode images into AVIF format. (Small and Efficient)
  farbfeld  Encode images into Farbfeld format. (Bitmapped)
  jpeg      Encode images into JPEG format. (Progressive-able)
  jpeg_xl   Encode images into JpegXL format. (Big but Lossless)
  mozjpeg   Encode images into JPEG format using MozJpeg codec. (RECOMMENDED and Small)
  oxipng    Encode images into PNG format using OxiPNG codec. (Progressive-able)
  png       Encode images into PNG format.
  ppm       Encode images into PPM format. (Bitmapped)
  qoi       Encode images into QOI format. (Trendy and Small)
  webp      Encode images into WebP format. (Lossless-able)
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### Basic optimization suitable for web

To optimize images with great defaults, you can simply call `rimage <command>`. For example:

```sh
rimage mozjpeg ./image.jpg
```

By default rimage will place output images right in place of precious images, resulting in overwrite if input and output has the same format. To change this behavior you can use this options:

```sh
# will place output images in `./output` directory, images may be overwritten if has the same name
rimage mozjpeg -d ./output ./image.jpg

# will rename all input files before processing with `@backup` suffix
rimage mozjpeg --backup ./image.jpg

# will place output images in ./output directory preserving folder structure
rimage mozjpeg -d ./output -r ./inner/image.jpg ./image.jpg
```

### Preprocessing

Rimage supports a preprocessing pipeline: resize, color quantization, and alpha premultiply
run before encoding. Operations execute in CLI argument order.

#### Resize

```sh
# Resize to fixed dimensions
rimage mozjpeg --resize 500x200 ./image.jpg

# Resize by width, preserving aspect ratio (200h for height)
rimage mozjpeg --resize 100w ./image.jpg
```

#### Quantization (color palette reduction)

`--quantization` reduces the number of distinct colors in the image. It is **not** a
substitute for `-q`/`--quality` — quantization limits the color palette, while
`-q` controls encoder compression.

```sh
# Quantize to 80% palette quality, then encode at default JPEG quality 75
rimage mozjpeg --quantization 80 ./image.jpg

# For best compression, combine with a lower quality value
rimage mozjpeg -q 50 --quantization 80 ./image.jpg
```

> **Note**: Using `--quantization` without lowering `-q` may produce files nearly as large
> as without quantization, because sharp palette boundaries (banding) are faithfully
> reproduced by the encoder at high quality settings.

#### Pipeline ordering

Preprocessing operations run in the order they appear on the command line:

```sh
# Quantize first, then resize to 64x64 (nearest filter)
rimage mozjpeg --quantization 80 --resize 64x64 --filter nearest ./image.jpg

# Resize first, then quantize
rimage mozjpeg --resize 64x64 --filter nearest --quantization 80 ./image.jpg
```

Note that `--filter` applies to all `--resize` invocations, and `--dithering`
applies to all `--quantization` invocations.

### Advanced options

If you want customize optimization you can provide additional options to encoders. For mozjpeg this options are valid:

```text
Options:
  -q, --quality <NUM>
          Quality, values 60-80 are recommended.

          [default: 75]

      --chroma_quality <NUM>
          Separate chrome quality.

      --baseline
          Set to use baseline encoding (by default is progressive).

      --no_optimize_coding
          Set to make files larger for no reason.

      --smoothing <NUM>
          Use MozJPEG's smoothing.

      --colorspace <COLOR>
          Set color space of JPEG being written.

          [default: ycbcr]
          [possible values: ycbcr, grayscale, rgb]

      --multipass
          Specifies whether multiple scans should be considered during trellis quantization.

      --subsample <PIX>
          Sets chroma subsampling.

      --qtable <TABLE>
          Use a specific quantization table.

          [default: NRobidoux]
          [possible values: AhumadaWatsonPeterson, AnnexK, Flat, KleinSilversteinCarney, MSSSIM, NRobidoux, PSNRHVS, PetersonAhumadaWatson, WatsonTaylorBorthwick]
```

For more info use `rimage help <command>`, e.g. `rimage help mozjpeg`

For library usage check [Docs.rs](https://docs.rs/rimage/latest/rimage/)

### List of supported Codecs

| Image Codecs | Decoder       | Encoder                 | NOTE                                                 |
| ------------ | ------------- | ----------------------- | ---------------------------------------------------- |
| avif         | libavif       | ravif                   | Common features only, Static only                    |
| bmp          | zune-bmp      | ❌                      | Input only                                           |
| farbfeld     | zune-farbfeld | zune-farbfeld           |                                                      |
| hdr          | zune-hdr      | zune-hdr                |                                                      |
| jpeg         | zune-jpeg     | mozjpeg or jpeg-encoder | Multifunctional when use mozjpeg encoder             |
| jpeg-xl      | jxl-oxide     | zune-jpegxl             | Lossless only                                        |
| png          | zune-png      | oxipng or zune-png      | Static only, Multifunctional when use oxipng encoder |
| ppm          | zune-ppm      | zune-ppm                |                                                      |
| psd          | zune-psd      | ❌                      | Input only                                           |
| qoi          | zune-qoi      | zune-qoi                |                                                      |
| tiff         | tiff          | ❌                      | Input only                                           |
| webp         | webp          | webp                    | Static only                                          |

### List of supported preprocessing options

- Resize
- Quantization
- Alpha premultiply

## List of supported mode for output info presenting

- No-progress (Shown on Default)
- Quiet (Show all msgs on Default)

## Example

This will crash:

```sh
rimage png "D:\example.jpg" -d "D:\desktop\" -s "suffix"
```

This will work as expected:

```sh
rimage png "D:\example.jpg" -d "D:\desktop" -s "suf test" # without trailing backslash

rimage png "D:\example.jpg" -s "suffix"  -d "D:\desktop\" # backslash at the end
```

## Known bugs & Warnings

- **Path end with `\` may cause rimage crashes** due to a cmd bug [#72653](https://github.com/rust-lang/rust/issues/72653).
- Mozjpeg's SIMD assembly optimization code has ABI compatibility issues with code generated by the Windows GNU toolchain (MinGW/GCC) in Release mode. This can cause the program to crash, so you **MUST use the MSVC toolchain for compilation** according to [rimage_gui#29](https://github.com/Mikachu2333/rimage_gui/issues/29).
- PSD is partially supported, img that with icc config file may result in wrong color.

## Build (Windows)

1. Clone the repository:

   ```pwsh
   git clone https://github.com/vlad-salone/rimage --depth=1
   cd rimage
   ```

2. Install MSVC toolchain (Windows):

   ```pwsh
   rustup default stable-x86_64-pc-windows-msvc
   ```

3. Install MSVC Build Tools (Windows):
   - Download and install from [Visual Studio 2026 Build Tools](https://visualstudio.microsoft.com/en-us/downloads/).
   - During installation, select "Desktop development with C++" workload.
   - OR, just use `choco install visualstudio2022-workload-vctools` to install it.

4. Install Perl:
   - Download and install from [Strawberry Perl](https://strawberryperl.com/).
   - OR, just use `choco install strawberryperl` to install it.

5. Install cmake (OPTIONAL if you use the MSVC bundled version):
    - Download and install from [CMake](https://cmake.org/download/).
    - OR, just use `choco install cmake` to install it.
    - OR, you can use the bundled cmake in MSVC, but please note that only 4.2.3+ could be used.

6. **RENAME** cmake from perl:
    - Go to `C:\Strawberry\c\bin` (Your Perl installation directory) and rename `cmake.exe` to `cmake.exe.bak` to avoid conflicts with cmake installed in step 5 (The bundled cmake in perl is outdated and would distrube build scripts).

7. Make sure `$PATH`
    - Make sure the cmake installed in step 5 is in your system `$PATH` and can be called from command line. You can check this by running `cmake --version` in your terminal, it should show the version of cmake you installed in step 5.
    - Make sure Perl is in your system `$PATH` and can be called from command line. You can check this by running `perl --version` in your terminal, it should show the version of Perl you installed in step 4.

8. Build / Test / Format the project:

    ```pwsh
    cargo build --release --all-features # BUILD
    cargo run --features=build-binary ... # TEST with args
    cargo clippy --all-features -- -D warnings # CHECK
    cargo fmt --all -- --check # FORMAT
    cargo test --workspace --all-features # TEST
    ```

## Contributing

Read the [contribution guide](CONTRIBUTING.md) for build instructions and guidelines.

## License

Rimage is dual-licensed under [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) and [MIT License](https://opensource.org/licenses/MIT). You can choose either license for your use.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

## Changelog

View the [Changelog](CHANGELOG.md) for version-specific changes.
