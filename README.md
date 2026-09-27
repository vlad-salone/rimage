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

# will keep the original image as `<name>@backup.<ext>` next to the input
rimage mozjpeg --backup ./image.jpg

# will place output images in ./output directory preserving folder structure
rimage mozjpeg -d ./output -r ./inner/image.jpg ./image.jpg
```

### Color fidelity and chroma subsampling

`rimage mozjpeg` lets MozJPEG choose the chroma subsampling automatically. At
typical quality levels it picks **2x2 (4:2:0)**, which discards three quarters
of the chroma resolution to make the file smaller. A side effect is that highly
saturated colors (for example a bright red) can come out visibly darker or
duller than the original.

If color fidelity matters more than file size, force 4:4:4 with:

```sh
rimage mozjpeg --subsample 1 ./image.jpg
```

`1` means 4:4:4 (no chroma subsampling), `2` means 4:2:0. The default
quantization table is `NRobidoux`; if you are comparing output with tools that
use the standard Annex K tables (such as ImageMagick), pass
`--qtable AnnexK` to make the comparison closer.

### File lists

To process many images without a long command line, create a UTF-8 text file named `file.list`
with one input file per line and pass it as an input:

```sh
rimage mozjpeg file.list
```

Blank lines are skipped, surrounding whitespace is ignored, and relative paths are resolved
against the current working directory. Glob patterns are supported on each line. When a
`file.list` is provided, all other input file arguments are ignored.

### Preprocessing

Rimage supports a preprocessing pipeline: resize, color quantization, and alpha premultiply
run before encoding. Operations execute in CLI argument order.

#### Resize

`--resize` accepts one of the following value forms. Unless a fixed `WxH` is
given, the aspect ratio is preserved:

| Form  | Meaning                                               | Example   | Result on 800x400 |
| ----- | ----------------------------------------------------- | --------- | ----------------- |
| `WxH` | Fixed width and height, aspect ratio is not preserved | `100x300` | `100x300`         |
| `Ww`  | Anchor on the width, height follows the aspect ratio  | `100w`    | `100x50`          |
| `hH`  | Anchor on the height, width follows the aspect ratio  | `200h`    | `400x200`         |
| `Ll`  | Longest side becomes `L`, the other side follows      | `1000l`   | `1000x500`        |
| `Ss`  | Shortest side becomes `S`, the other side follows     | `500s`    | `1000x500`        |
| `@M`  | Scale by the multiplier `M`                           | `@1.5`    | `1200x600`        |
| `P%`  | Scale to `P` percent of the source                    | `50%`     | `400x200`         |

```sh
# Fixed dimensions: the image is resized to exactly 100x200
rimage mozjpeg --resize 100x200 ./image.jpg     # 800x400 -> 100x200

# Anchor on one side, the other side follows the aspect ratio
rimage mozjpeg --resize 100w ./image.jpg        # 800x400 -> 100x50
rimage mozjpeg --resize 200h ./image.jpg        # 800x400 -> 400x200

# Longest / shortest side: the anchor is chosen per image, so portrait and
# landscape images in the same batch end up with a consistent size
rimage mozjpeg --resize 1000l ./landscape.jpg   # 800x400 -> 1000x500
rimage mozjpeg --resize 1000l ./portrait.jpg    # 400x800 -> 500x1000
rimage mozjpeg --resize 500s ./landscape.jpg    # 800x400 -> 1000x500
rimage mozjpeg --resize 500s ./portrait.jpg     # 400x800 -> 500x1000

# Multiplier and percentage
rimage mozjpeg --resize @2 ./image.jpg          # 800x400 -> 1600x800
rimage mozjpeg --resize @0.5 ./image.jpg        # 800x400 -> 400x200
rimage mozjpeg --resize 50% ./image.jpg         # 800x400 -> 400x200
rimage mozjpeg --resize 150% ./image.jpg        # 800x400 -> 1200x600
```

Passing `--resize` several times chains the values. Each value maps the size
the previous resize produced, so the order of the values matters:

```sh
# 100x400 first, then the shortest side of that intermediate result
rimage mozjpeg --resize 100x400 --resize 200s ./image.jpg   # 800x400 -> 200x800

# Longest side first, then half of the intermediate result
rimage mozjpeg --resize 1000l --resize 50% ./image.jpg      # 800x400 -> 500x250
```

The direction flags restrict each resize step to shrinking or growing only.
`--reduce-only` is an alias for `--no-upscale` (never grow) and `--enlarge-only`
is an alias for `--no-downscale` (never shrink):

```sh
# Shrink images whose longest side is above 1000px, leave the rest untouched
rimage mozjpeg --resize 1000l --reduce-only ./small.jpg     # 800x400 -> 800x400
rimage mozjpeg --resize 1000l --reduce-only ./big.jpg       # 2000x1000 -> 1000x500

# Grow images whose longest side is below 1000px, leave the rest untouched
rimage mozjpeg --resize 1000l --enlarge-only ./small.jpg    # 800x400 -> 1000x500
rimage mozjpeg --resize 1000l --enlarge-only ./big.jpg      # 2000x1000 -> 2000x1000

# A step that is not allowed by the flags is skipped, and the chain continues
# from the size the image actually has
rimage mozjpeg --resize 2000l --resize 50% --reduce-only ./image.jpg  # 800x400 -> 400x200
```

> **Note**: Passing both `--reduce-only` and `--enlarge-only` leaves no direction to resize
> in, so every image keeps its original size. The order of the two flags makes no difference.

`--filter` selects the resampling filter applied to every `--resize` step
(default `lanczos3`). Available filters: `nearest`, `box`, `bilinear`,
`hamming`, `catmull-rom`, `mitchell`, `lanczos3`.

```sh
rimage mozjpeg --resize 1000l --filter nearest ./image.jpg
```

SVG inputs are resized through the same `--resize` parameter. Instead of
rasterizing the SVG at its intrinsic size and then resampling with
`fast_image_resize`, the SVG is rendered vectorly with `resvg` directly at the
final target size. Upscaling therefore keeps the vector quality of the source,
and chained `--resize` values compose for SVG exactly as they do for raster
images. `--filter` selects the raster resampling filter used by
`fast_image_resize`; it is still accepted for SVG inputs (when `--resize` is
present) but has no effect on vector SVG rendering.

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

### SVG input

SVG files (and gzipped `.svgz`) are rendered through `resvg` before encoding,
so an SVG can be fed to any output format. By default the SVG renders at its
intrinsic size — taken from its `width`/`height`, falling back to the
`viewBox`.

Use `--resize` to control the rasterization size. The SVG is rasterized
directly at the requested size, so sizing is **lossless at any scale**: edges,
gradients and text stay exactly as sharp when you enlarge it. Chained
`--resize` values compose for SVG exactly as they do for raster images, and
the direction flags (`--reduce-only`, `--enlarge-only`) work for SVG too.

```sh
# Render a 100x50 SVG at 2x and encode it as PNG
rimage png --resize @2 ./logo.svg

# Render at a fixed width for a 2x-density asset, height follows
rimage mozjpeg --resize 800w ./logo.svg
```

`--resize` affects `.svg`/`.svgz` inputs and raster images alike. `--filter`
is a raster resampling filter and has no effect on vector SVG rendering.

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
| avif         | dav1d         | ravif                   | Common features only, Static only                    |
| bmp          | zune-bmp      | ❌                      | Input only                                           |
| farbfeld     | zune-farbfeld | zune-farbfeld           |                                                      |
| hdr          | zune-hdr      | zune-hdr                |                                                      |
| jpeg         | zune-jpeg     | mozjpeg or jpeg-encoder | Multifunctional when use mozjpeg encoder             |
| jpeg-xl      | jxl-oxide     | zune-jpegxl             | Lossless only, Static only                           |
| png          | zune-png      | oxipng or zune-png      | Static only, Multifunctional when use oxipng encoder |
| ppm          | zune-ppm      | zune-ppm                |                                                      |
| psd          | zune-psd      | ❌                      | Input only                                           |
| qoi          | zune-qoi      | zune-qoi                |                                                      |
| svg          | resvg         | ❌                      | Input only                                           |
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
   - OR, just use `choco install visualstudio2026-workload-vctools` to install it.

4. Install cmake (OPTIONAL if you use the MSVC bundled version):
    - Download and install from [CMake](https://cmake.org/download/).
    - OR, just use `choco install cmake` to install it.
    - OR, you can use the bundled cmake in MSVC, but please note that only **4.2.3** + could be used.

5. Install nasm (for the mozjpeg encoder's SIMD code):
   - Download and install from [nasm](https://www.nasm.us/).
   - OR, just use `choco install nasm` to install it.

6. Install dav1d through [vcpkg](https://vcpkg.io/) (the AVIF decoder links a
   static dav1d found through pkg-config):

    ```pwsh
    vcpkg install dav1d:x64-windows-static pkgconf:x64-windows

    # point the build at pkgconf and the dav1d package
    $env:PKG_CONFIG = "$env:VCPKG_ROOT\installed\x64-windows\tools\pkgconf\pkgconf.exe"
    $env:PKG_CONFIG_PATH = "$env:VCPKG_ROOT\installed\x64-windows-static\lib\pkgconfig"
    ```

    Use `x86-windows-static` instead of `x64-windows-static` when building for
    the i686 target. To persist the two environment variables, add them with
    `setx` or through the system settings.

7. Make sure cmake is in your system `$PATH` and can be called from command
   line. You can check this by running `cmake --version` in your terminal, it
   should show the version of cmake you installed in step 4.

9. Build / Test / Format the project:

    ```pwsh
    # build the library and the CLI binary
    cargo build --all-features

    # run the binary during development
    cargo run --all-features -- mozjpeg ./image.jpg

    # unit and integration tests (install cargo-nextest first, original `cargo test` is also acceptable)
    cargo nextest run --release --all-features

    # doc tests; nextest does not run them
    cargo test --doc --release --all-features

    # format and check for formatting issues
    cargo clippy --all-features -- -D warnings
    cargo fmt --all -- --check
    ```

## Contributing

Read the [contribution guide](CONTRIBUTING.md) for build instructions and guidelines.

## License

Rimage is dual-licensed under [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) and [MIT License](https://opensource.org/licenses/MIT). You can choose either license for your use.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

## Changelog

View the [Changelog](CHANGELOG.md) for version-specific changes.
