//! Bounded-memory character-art conversion. No terminal writes or process exits.
//! Input pixels are RGBA; charset order is dark to bright for a dark terminal.
//! ```
//! use charart::{Backend, Converter, Options};
//! let pixels = image::RgbaImage::new(8, 8);
//! let mut converter = Converter::new(Options {
//!     backend: Backend::Cpu, width: Some(4), ..Default::default()
//! })?;
//! let frame = converter.convert(&pixels)?;
//! assert_eq!((frame.width, frame.height), (4, 2));
//! # Ok::<(), charart::Error>(())
//! ```
#[cfg(feature = "cuda")]
mod cuda;
mod mapping;
pub mod media;
pub mod output;
mod pixels;
mod sampling;
mod tone;
pub mod tuning;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
#[cfg(feature = "cuda")]
use std::borrow::Cow;
use std::time::Instant;
pub use tone::Tone;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("CUDA unavailable: {0}")]
    Cuda(String),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Backend {
    Auto,
    Cpu,
    Cuda,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Ascii,
    Braille,
    Halfblock,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dither {
    #[default]
    None,
    Bayer,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub scale: f64,
    /// Character-cell width divided by height (usually 0.5).
    pub cell_aspect: f64,
    pub charset: String,
    pub invert: bool,
    /// None uses per-frame Otsu threshold. ASCII uses threshold as its midpoint.
    pub threshold: Option<u8>,
    pub quantize: u32,
    pub background: [u8; 3],
    pub mode: Mode,
    pub backend: Backend,
    pub dither: Dither,
    pub tone: Tone,
    /// Use foreground/background color pairs for halfblocks; false uses threshold bits.
    pub halfblock_color: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            width: Some(80),
            height: None,
            scale: 1.,
            cell_aspect: 0.5,
            charset: " .:-=+*#%@".into(),
            invert: false,
            threshold: None,
            quantize: 256,
            background: [0; 3],
            mode: Mode::Ascii,
            backend: Backend::Auto,
            dither: Dither::None,
            tone: Tone::default(),
            halfblock_color: false,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        self.tone.validate()?;
        if self.mode == Mode::Halfblock && self.halfblock_color && self.invert {
            return Err(Error::Config("invert requires thresholded halfblocks or ASCII/Braille; use tone controls for color halfblocks".into()));
        }
        if self.mode == Mode::Halfblock && self.halfblock_color && self.dither != Dither::None {
            return Err(Error::Config("colored halfblocks use continuous colors; choose --dither none or monochrome output".into()));
        }
        let chars: Vec<_> = self.charset.chars().collect();
        if chars.len() < 2
            || chars.len() > 256
            || chars.iter().any(|c| c.is_control() || c.width() != Some(1))
        {
            return Err(Error::Config(
                "charset needs 2..256 printable single-column characters".into(),
            ));
        }
        if !self.scale.is_finite()
            || self.scale <= 0.
            || !self.cell_aspect.is_finite()
            || self.cell_aspect <= 0.
            || !(2..=256).contains(&self.quantize)
            || self.width == Some(0)
            || self.height == Some(0)
        {
            return Err(Error::Config(
                "positive finite scale/aspect/dimensions and quantize 2..256 required".into(),
            ));
        }
        Ok(())
    }
    pub fn dimensions(&self, w: u32, h: u32) -> Result<(u32, u32)> {
        self.validate()?;
        self.dimensions_validated(w, h)
    }
    fn dimensions_validated(&self, w: u32, h: u32) -> Result<(u32, u32)> {
        if w == 0 || h == 0 {
            return Err(Error::Config("empty image".into()));
        }
        let ratio = h as f64 / w as f64 * self.cell_aspect;
        let (x, y) = match (self.width, self.height) {
            (Some(x), Some(y)) => (x as f64, y as f64),
            (Some(x), None) => (x as f64, x as f64 * ratio),
            (None, Some(y)) => (y as f64 / ratio, y as f64),
            (None, None) => (w as f64, h as f64 * self.cell_aspect),
        };
        let x = (x * self.scale).round().max(1.);
        let y = (y * self.scale).round().max(1.);
        if x > 4096. || y > 4096. || x * y > 1_048_576. {
            return Err(Error::Config(
                "output exceeds 4096 per axis or 1,048,576 cells".into(),
            ));
        }
        Ok((x as u32, y as u32))
    }
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Cell {
    pub glyph: char,
    pub rgb: [u8; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monochrome: Option<char>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostics {
    pub backend: String,
    pub fallback_reason: Option<String>,
    pub threshold: u8,
    pub processing_ms: f64,
    /// CUDA events around a warmed launch, excluding copies and JIT; CPU is None.
    pub kernel_ms: Option<f32>,
    pub mode: Mode,
    pub dither: Dither,
    pub tone_range: Option<[u8; 2]>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Art {
    pub width: u32,
    pub height: u32,
    pub cells: Vec<Cell>,
    pub diagnostics: Diagnostics,
}
/// Stateful converter: caches the CUDA context; auto falls back with a diagnostic.
pub struct Converter {
    options: Options,
    reason: Option<String>,
    mapper: mapping::Mapper,
    sampler: sampling::Sampler,
    transform: pixels::Transform,
    packed: Vec<u32>,
    tone_lut: Option<[u8; 256]>,
    tone_range: Option<[u8; 2]>,
    #[cfg(feature = "cuda")]
    gpu: Option<cuda::Gpu>,
}
impl Converter {
    pub fn new(options: Options) -> Result<Self> {
        options.validate()?;
        let mut reason = None;
        #[cfg(feature = "cuda")]
        let gpu = if options.backend != Backend::Cpu {
            match cuda::Gpu::new() {
                Ok(g) => Some(g),
                Err(e) => {
                    if options.backend == Backend::Cuda {
                        return Err(Error::Cuda(e.to_string()));
                    }
                    reason = Some(e.to_string());
                    None
                }
            }
        } else {
            None
        };
        #[cfg(not(feature = "cuda"))]
        if options.backend != Backend::Cpu {
            if options.backend == Backend::Cuda {
                return Err(Error::Cuda(
                    "build with --features cuda on a supported CUDA host".into(),
                ));
            }
            reason = Some("binary built without the cuda feature".into());
        }
        let mapper = mapping::Mapper::new(&options.charset);
        let tone_lut = if options.tone != Tone::default() && !options.tone.auto_contrast {
            Some(options.tone.lut([0, 255]))
        } else {
            None
        };
        Ok(Self {
            mapper,
            sampler: sampling::Sampler::default(),
            transform: pixels::Transform::new(options.background, options.quantize),
            packed: Vec::new(),
            tone_lut,
            tone_range: None,
            options,
            reason,
            #[cfg(feature = "cuda")]
            gpu,
        })
    }
    /// Allocate an owned result; converter scratch storage is reused between frames.
    pub fn convert(&mut self, image: &RgbaImage) -> Result<Art> {
        let mut art = Art {
            width: 0,
            height: 0,
            cells: Vec::new(),
            diagnostics: Diagnostics {
                backend: String::new(),
                fallback_reason: None,
                threshold: 0,
                processing_ms: 0.,
                kernel_ms: None,
                mode: self.options.mode,
                dither: self.options.dither,
                tone_range: None,
            },
        };
        self.convert_into(image, &mut art)?;
        Ok(art)
    }
    /// Replace an existing result, reusing its cell and diagnostic storage.
    /// On a conversion error the supplied result remains unchanged.
    pub fn convert_into(&mut self, image: &RgbaImage, art: &mut Art) -> Result<()> {
        let start = Instant::now();
        let o = &self.options;
        let (w, h) = o.dimensions_validated(image.width(), image.height())?;
        let (sx, sy) = match o.mode {
            Mode::Ascii => (1, 1),
            Mode::Braille => (2, 4),
            Mode::Halfblock => (1, 2),
        };
        let mut kernel_ms = None;
        let mut backend = "cpu";
        let mut needs_cpu = true;
        #[cfg(feature = "cuda")]
        if let Some(g) = &mut self.gpu {
            let sampled = if image.dimensions() == (w * sx, h * sy) {
                Cow::Borrowed(image)
            } else {
                Cow::Owned(self.sampler.resize(image, w * sx, h * sy))
            };
            match g.process(&sampled, o.background, o.quantize) {
                Ok((packed, k)) => {
                    self.packed = packed;
                    kernel_ms = Some(k);
                    backend = "cuda";
                    needs_cpu = false;
                }
                Err(e) => {
                    if o.backend == Backend::Cuda {
                        return Err(Error::Cuda(e.to_string()));
                    }
                    self.reason = Some(e.to_string());
                    self.gpu = None;
                }
            }
        }
        let mut tone_applied = false;
        if needs_cpu {
            let transform = &self.transform;
            if let Some(lut) = &self.tone_lut {
                self.sampler
                    .map_into(image, w * sx, h * sy, &mut self.packed, |p| {
                        pixels::tone(transform.pack(p), lut)
                    });
                tone_applied = true;
            } else {
                self.sampler
                    .map_into(image, w * sx, h * sy, &mut self.packed, |p| {
                        transform.pack(p)
                    });
            }
        }
        // These values change only in CUDA-enabled builds.
        let _ = (&mut kernel_ms, &mut backend, &mut needs_cpu);
        let packed = &mut self.packed;
        if o.tone.auto_contrast && self.tone_lut.is_none() {
            let mut hist = [0u64; 256];
            for p in packed.iter() {
                hist[(p >> 24) as usize] += 1;
            }
            let range = tone::percentiles(&hist, packed.len() as u64);
            self.tone_lut = Some(o.tone.lut(range));
            self.tone_range = Some(range);
        }
        if !tone_applied && let Some(lut) = &self.tone_lut {
            for p in packed.iter_mut() {
                *p = pixels::tone(*p, lut);
            }
        }
        let threshold = o.threshold.unwrap_or_else(|| otsu(packed));
        self.mapper
            .cells_into(packed, w, threshold, o, &mut art.cells);
        art.width = w;
        art.height = h;
        let diagnostics = &mut art.diagnostics;
        diagnostics.backend.clear();
        diagnostics.backend.push_str(backend);
        diagnostics.fallback_reason.clone_from(&self.reason);
        diagnostics.threshold = threshold;
        diagnostics.kernel_ms = kernel_ms;
        diagnostics.mode = o.mode;
        diagnostics.dither = o.dither;
        diagnostics.tone_range = self.tone_range;
        diagnostics.processing_ms = start.elapsed().as_secs_f64() * 1000.;
        Ok(())
    }
}
/// Fused integer reference. RGBA composite rounds to nearest; luma uses 77/150/29.
pub fn cpu_pixels(image: &RgbaImage, bg: [u8; 3], q: u32) -> Result<Vec<u32>> {
    if !(2..=256).contains(&q) {
        return Err(Error::Config("quantize must be 2..256".into()));
    }
    Ok(image
        .pixels()
        .map(|p| {
            let a = p[3] as u32;
            let c: [u32; 3] =
                std::array::from_fn(|i| (p[i] as u32 * a + bg[i] as u32 * (255 - a) + 127) / 255);
            let l = (77 * c[0] + 150 * c[1] + 29 * c[2] + 128) / 256;
            let c = if q == 256 {
                c
            } else {
                c.map(|v| ((v * (q - 1) + 127) / 255 * 255 + (q - 1) / 2) / (q - 1))
            };
            c[0] + c[1] * 256 + c[2] * 65536 + l * 16777216
        })
        .collect())
}
fn otsu(pixels: &[u32]) -> u8 {
    let mut hist = [0u64; 256];
    for p in pixels {
        hist[(p >> 24) as usize] += 1;
    }
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, n)| i as f64 * *n as f64)
        .sum();
    let mut weight = 0.;
    let mut partial = 0.;
    let mut best = -1.;
    let mut threshold = 127;
    for (i, n) in hist.iter().enumerate() {
        weight += *n as f64;
        partial += i as f64 * *n as f64;
        let other = pixels.len() as f64 - weight;
        if weight == 0. || other == 0. {
            continue;
        }
        let delta = partial / weight - (sum - partial) / other;
        let score = weight * other * delta * delta;
        if score > best {
            best = score;
            threshold = i as u8;
        }
    }
    threshold
}
