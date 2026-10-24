# CharArt.rs

A Rust CLI and library that converts images, GIFs, and videos into character art.
Supports ASCII, Braille, half blocks, monochrome and color output, and automatic tuning.
The crate and command are both named `charart`.

## Build and dependencies

Requires Rust 1.89 or later (tested with 1.99). The CPU build does not require CUDA.

```sh
cargo install --path . --locked
charart --help
```

The first build needs network access to fetch dependencies.
Images and GIFs need no external decoder. Videos require `ffmpeg` and `ffprobe` on `PATH`.
Install with `brew install ffmpeg` on macOS or `sudo apt install ffmpeg` on Ubuntu.
For Windows, see the [FFmpeg download page](https://ffmpeg.org/download.html). FFmpeg is not bundled.

## Usage

```sh
charart photo.png --auto                         # Tune settings for the image
charart photo.png --width 80 --mode braille       # Choose width and rendering mode
charart photo.png --charset ' .oO@' --invert      # Customize characters and invert brightness
charart photo.png --color -o art.html            # Save color output
charart photo.png --color -o header.lua          # Export for alpha-nvim
charart animation.gif --play --loop --speed 1.5  # Play in the terminal
charart clip.mp4 --color --play
charart photo.png --analyze -o settings.json     # Save automatically chosen settings
charart photo.png --profile settings.json -o art.txt
```

Inputs: PNG, JPEG, BMP, PNM, WebP, GIF, and videos supported by FFmpeg.
Outputs: text, ANSI, HTML, Lua, and JSONL, selected by the output extension or `--format`.
Use `--frames 1` when exporting a GIF or video to HTML or Lua.
Width defaults to the terminal width. Height, scale, cell aspect ratio, threshold, background,
color quantization, dithering, brightness, contrast, and gamma are configurable. See `--help` for details.

## Library

```rust
use charart::{Backend, Converter, Options, output::{self, Format}};
let image = image::open("photo.png")?.into_rgba8();
let mut converter = Converter::new(Options { backend: Backend::Cpu, ..Default::default() })?;
let art = converter.convert(&image)?;
let text = output::render(&art, Format::Text, false);
```

The library does not write to standard output or exit the process.

## CUDA (optional)

Uses cuTile from [NVIDIA/cuda-rust](https://github.com/NVIDIA/cuda-rust) for alpha compositing,
luminance, and color quantization. Requires Linux, a supported NVIDIA GPU (sm_80 or later),
the NVIDIA driver, libclang, and the CUDA Toolkit with `tileiras`.
Toolkit 13.3 or later is recommended (13.4 or later for sm_107).

```sh
export CUDA_TOOLKIT_PATH=/usr/local/cuda-13.3
cargo build --release --locked --features cuda
./target/release/charart photo.png --backend cuda --color
```

Select with `--backend auto|cpu|cuda`. If CUDA is unavailable, `auto` reports the reason and
falls back to the CPU; explicitly selecting `cuda` returns an error.
Execution and performance on physical GPU hardware have not been verified.

## Distribution and limits

Pushing a `v<version>` tag builds CPU binaries for Linux x86_64/ARM64, macOS Intel/Apple Silicon,
and Windows x86_64. The workflow can also be run manually for an existing tag.
Binaries, checksums, and dependency licenses are stored as Actions artifacts for 30 days.

Resizing uses nearest-neighbor sampling. Playback has no audio. APNG and animated WebP use only the first image.
Output is limited to 4,096 cells per axis and 1,048,576 cells in total; input is limited to 33,554,432 pixels.

[MIT license](LICENSE). Dependency license texts are included with distributed binaries.
