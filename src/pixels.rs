//! Integer color transform cached once per converter. Luma precedes RGB quantization.
pub(crate) struct Transform {
    background: [u32; 3],
    quantize: Option<[u8; 256]>,
}
impl Transform {
    pub(crate) fn new(background: [u8; 3], levels: u32) -> Self {
        Self {
            background: background.map(u32::from),
            quantize: (levels != 256).then(|| {
                std::array::from_fn(|v| {
                    ((v as u32 * (levels - 1) + 127) / 255 * 255 + (levels - 1) / 2)
                        .div_euclid(levels - 1) as u8
                })
            }),
        }
    }
    #[inline]
    pub(crate) fn pack(&self, pixel: [u8; 4]) -> u32 {
        let a = u32::from(pixel[3]);
        let mut rgb = if a == 255 {
            [
                u32::from(pixel[0]),
                u32::from(pixel[1]),
                u32::from(pixel[2]),
            ]
        } else {
            std::array::from_fn(|c| {
                (u32::from(pixel[c]) * a + self.background[c] * (255 - a) + 127) / 255
            })
        };
        let luma = (77 * rgb[0] + 150 * rgb[1] + 29 * rgb[2] + 128) / 256;
        if let Some(lut) = &self.quantize {
            rgb = rgb.map(|v| u32::from(lut[v as usize]));
        }
        rgb[0] | rgb[1] << 8 | rgb[2] << 16 | luma << 24
    }
}
#[inline]
pub(crate) fn tone(pixel: u32, lut: &[u8; 256]) -> u32 {
    u32::from(lut[(pixel & 255) as usize])
        | u32::from(lut[(pixel >> 8 & 255) as usize]) << 8
        | u32::from(lut[(pixel >> 16 & 255) as usize]) << 16
        | u32::from(lut[(pixel >> 24) as usize]) << 24
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_transform_matches_integer_reference() {
        for levels in [2, 3, 16, 255, 256] {
            for bg in [[0, 0, 0], [12, 34, 56], [255, 128, 17]] {
                let transform = Transform::new(bg, levels);
                for alpha in 0..=255 {
                    for value in 0u8..=255 {
                        let p = [value, 255 - value, value.wrapping_mul(71), alpha];
                        let c: [u32; 3] = std::array::from_fn(|i| {
                            (p[i] as u32 * alpha as u32 + bg[i] as u32 * (255 - alpha as u32) + 127)
                                / 255
                        });
                        let l = (77 * c[0] + 150 * c[1] + 29 * c[2] + 128) / 256;
                        let q = c.map(|v| {
                            ((v * (levels - 1) + 127) / 255 * 255 + (levels - 1) / 2) / (levels - 1)
                        });
                        assert_eq!(
                            transform.pack(p),
                            q[0] + q[1] * 256 + q[2] * 65536 + l * 16777216
                        );
                    }
                }
            }
        }
    }
}
