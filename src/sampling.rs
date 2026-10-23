//! Cached center-coordinate nearest sampling. Same f32 coordinates as image 0.25.
use image::RgbaImage;
#[derive(Default)]
pub(crate) struct Sampler {
    dimensions: Option<(u32, u32, u32, u32)>,
    xs: Vec<u32>,
    ys: Vec<u32>,
}
impl Sampler {
    /// Fuse sampling into the consumer; no intermediate RGBA image on the CPU.
    pub(crate) fn map_into<T>(
        &mut self,
        src: &RgbaImage,
        w: u32,
        h: u32,
        out: &mut Vec<T>,
        mut map: impl FnMut([u8; 4]) -> T,
    ) {
        out.clear();
        out.reserve((w * h) as usize);
        if src.dimensions() == (w, h) {
            out.extend(src.pixels().map(|p| map(p.0)));
            return;
        }
        self.prepare(src, w, h);
        let stride = src.width() as usize;
        let pixels = src.as_raw().as_chunks::<4>().0;
        for &y in &self.ys {
            let row = &pixels[y as usize * stride..(y as usize + 1) * stride];
            out.extend(self.xs.iter().map(|&x| map(row[x as usize])));
        }
    }
    fn prepare(&mut self, src: &RgbaImage, w: u32, h: u32) {
        let dims = (src.width(), src.height(), w, h);
        if self.dimensions != Some(dims) {
            self.xs.clear();
            self.ys.clear();
            self.xs.extend((0..w).map(|x| {
                (((x as f32 + 0.5) * (src.width() as f32 / w as f32)) as u32).min(src.width() - 1)
            }));
            self.ys.extend((0..h).map(|y| {
                (((y as f32 + 0.5) * (src.height() as f32 / h as f32)) as u32).min(src.height() - 1)
            }));
            self.dimensions = Some(dims);
        }
    }
    #[cfg(feature = "cuda")]
    pub(crate) fn resize(&mut self, src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
        self.prepare(src, w, h);
        let mut bytes = Vec::with_capacity((w * h) as usize * 4);
        for &y in &self.ys {
            for &x in &self.xs {
                bytes.extend_from_slice(&src.get_pixel(x, y).0);
            }
        }
        RgbaImage::from_raw(w, h, bytes).expect("validated dimensions")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_reference_for_random_dimensions_and_alpha() {
        let mut seed = 19u32;
        let mut sampler = Sampler::default();
        for _ in 0..300 {
            let mut next = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                seed
            };
            let (sw, sh, w, h) = (
                next() % 257 + 1,
                next() % 129 + 1,
                next() % 181 + 1,
                next() % 97 + 1,
            );
            let image = RgbaImage::from_fn(sw, sh, |_, _| image::Rgba(next().to_le_bytes()));
            let expected =
                image::imageops::resize(&image, w, h, image::imageops::FilterType::Nearest);
            let mut actual = Vec::new();
            sampler.map_into(&image, w, h, &mut actual, |p| p);
            #[cfg(feature = "cuda")]
            assert_eq!(sampler.resize(&image, w, h).as_raw(), expected.as_raw());
            let actual: Vec<_> = actual.into_iter().flatten().collect();
            assert_eq!(actual, *expected.as_raw(), "{sw}x{sh} -> {w}x{h}");
        }
    }
}
