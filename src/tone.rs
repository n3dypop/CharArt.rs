//! Deterministic tone LUTs shared by CPU and CUDA postprocessing.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tone {
    /// Additive normalized brightness, -1..1.
    pub brightness: f32,
    /// Contrast around middle gray, 0..4.
    pub contrast: f32,
    /// Output = input^(1/gamma), 0.1..5. Above 1 brightens.
    pub gamma: f32,
    /// Percentile stretch, frozen on the first converted frame.
    pub auto_contrast: bool,
}
impl Default for Tone {
    fn default() -> Self {
        Self {
            brightness: 0.,
            contrast: 1.,
            gamma: 1.,
            auto_contrast: false,
        }
    }
}
impl Tone {
    pub fn validate(&self) -> Result<()> {
        if !self.brightness.is_finite()
            || !(-1.0..=1.).contains(&self.brightness)
            || !self.contrast.is_finite()
            || !(0.0..=4.).contains(&self.contrast)
            || !self.gamma.is_finite()
            || !(0.1..=5.).contains(&self.gamma)
        {
            return Err(Error::Config(
                "brightness -1..1, contrast 0..4 and gamma 0.1..5 must be finite".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn lut(&self, range: [u8; 2]) -> [u8; 256] {
        std::array::from_fn(|i| {
            let x = if range[1] > range[0] {
                (i as f32 - range[0] as f32) / (range[1] - range[0]) as f32
            } else {
                i as f32 / 255.
            };
            let x = ((x.clamp(0., 1.) - 0.5) * self.contrast + 0.5 + self.brightness).clamp(0., 1.);
            (x.powf(1. / self.gamma) * 255.).round() as u8
        })
    }
}
pub(crate) fn percentiles(hist: &[u64; 256], total: u64) -> [u8; 2] {
    let mut cumulative = 0;
    let mut lo = None;
    let mut hi = 255;
    for (i, n) in hist.iter().enumerate() {
        cumulative += n;
        if lo.is_none() && cumulative > total / 100 {
            lo = Some(i as u8);
        }
        if cumulative >= total.saturating_mul(99).div_ceil(100) {
            hi = i as u8;
            break;
        }
    }
    let lo = lo.unwrap_or(0);
    // Uniform/near-uniform scenes should keep their original brightness.
    if hi.saturating_sub(lo) < 8 {
        [0, 255]
    } else {
        [lo, hi]
    }
}
