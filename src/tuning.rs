//! Separate, deterministic analysis -> profile -> conversion flow. No IO or printing.
use crate::{Dither, Error, Mode, Result, Tone};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub dither: Dither,
    pub tone: Tone,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub samples: u64,
    pub mean_luma: f64,
    pub p05: u8,
    pub p95: u8,
    pub mean_chroma: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub settings: Settings,
    pub analysis: Analysis,
    pub reasons: Vec<String>,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(Error::Config("unsupported tuning profile version".into()));
        }
        self.settings.tone.validate()?;
        if self.analysis.samples == 0
            || !self.analysis.mean_luma.is_finite()
            || !(0.0..=255.).contains(&self.analysis.mean_luma)
            || !self.analysis.mean_chroma.is_finite()
            || !(0.0..=255.).contains(&self.analysis.mean_chroma)
            || self.analysis.p05 > self.analysis.p95
        {
            return Err(Error::Config("invalid profile analysis".into()));
        }
        Ok(())
    }
    pub fn from_json(json: &str) -> Result<Self> {
        let profile: Self = serde_json::from_str(json)
            .map_err(|e| Error::Config(format!("invalid tuning profile: {e}")))?;
        profile.validate()?;
        Ok(profile)
    }
}
/// Analyze at most 256x256 evenly spaced RGBA samples, after background composition.
/// A video caller should call this once on the first frame and freeze the settings.
pub fn analyze(image: &RgbaImage, background: [u8; 3], color_output: bool) -> Result<Profile> {
    if image.width() == 0 || image.height() == 0 {
        return Err(Error::Config("cannot analyze an empty image".into()));
    }
    let w = image.width().min(256);
    let h = image.height().min(256);
    let mut hist = [0u64; 256];
    let mut sum = 0u64;
    let mut chroma = 0u64;
    for y in 0..h {
        for x in 0..w {
            let sx = ((2 * x as u64 + 1) * image.width() as u64 / (2 * w as u64))
                .min(image.width() as u64 - 1) as u32;
            let sy = ((2 * y as u64 + 1) * image.height() as u64 / (2 * h as u64))
                .min(image.height() as u64 - 1) as u32;
            let p = image.get_pixel(sx, sy);
            let a = p[3] as u32;
            let c: [u32; 3] = std::array::from_fn(|i| {
                (p[i] as u32 * a + background[i] as u32 * (255 - a) + 127) / 255
            });
            let l = (77 * c[0] + 150 * c[1] + 29 * c[2] + 128) / 256;
            hist[l as usize] += 1;
            sum += l as u64;
            chroma += (c.iter().max().unwrap() - c.iter().min().unwrap()) as u64;
        }
    }
    let samples = w as u64 * h as u64;
    let mut cumulative = 0;
    let mut lo = None;
    let mut hi = 255;
    for (i, n) in hist.iter().enumerate() {
        cumulative += n;
        if lo.is_none() && cumulative >= samples.div_ceil(20) {
            lo = Some(i as u8);
        }
        if cumulative >= samples.saturating_mul(19).div_ceil(20) {
            hi = i as u8;
            break;
        }
    }
    let analysis = Analysis {
        samples,
        mean_luma: sum as f64 / samples as f64,
        p05: lo.unwrap_or(0),
        p95: hi,
        mean_chroma: chroma as f64 / samples as f64,
    };
    let span = analysis.p95 - analysis.p05;
    let mode = if color_output && analysis.mean_chroma > 8. {
        Mode::Halfblock
    } else {
        Mode::Braille
    };
    let dither = if mode == Mode::Braille && span >= 16 {
        Dither::Bayer
    } else {
        Dither::None
    };
    let mut tone = Tone::default();
    if span >= 8 {
        tone.auto_contrast = span < 100;
        tone.gamma = if analysis.mean_luma < 80. {
            1.25
        } else if analysis.mean_luma > 180. {
            0.85
        } else {
            1.
        };
    }
    let reasons = vec![
        format!(
            "mode={mode:?}: color output={color_output}, mean chroma={:.1}",
            analysis.mean_chroma
        ),
        format!(
            "dither={dither:?}: p05..p95={}..{}",
            analysis.p05, analysis.p95
        ),
        format!(
            "gamma={}, auto contrast={}; fixed for this stream",
            tone.gamma, tone.auto_contrast
        ),
    ];
    Ok(Profile {
        schema_version: 1,
        settings: Settings { mode, dither, tone },
        analysis,
        reasons,
    })
}
