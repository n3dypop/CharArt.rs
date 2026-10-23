//! Mode-specific cell assembly. The threshold curve is cached for ASCII frames.
use crate::{Cell, Dither, Mode, Options};
const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
const HALF: [char; 4] = [' ', '▀', '▄', '█'];
pub(crate) struct Mapper {
    chars: Vec<char>,
    ascii: [char; 256],
    threshold: Option<u8>,
}
impl Mapper {
    pub(crate) fn new(charset: &str) -> Self {
        Self {
            chars: charset.chars().collect(),
            ascii: [' '; 256],
            threshold: None,
        }
    }
    pub(crate) fn cells_into(
        &mut self,
        packed: &[u32],
        width: u32,
        threshold: u8,
        options: &Options,
        cells: &mut Vec<Cell>,
    ) {
        if options.mode == Mode::Ascii && self.threshold != Some(threshold.clamp(1, 254)) {
            let t = u32::from(threshold.clamp(1, 254));
            self.ascii = std::array::from_fn(|l| {
                let l = l as u32;
                let value = if l <= t {
                    l * 127 / t
                } else {
                    128 + (l - t) * 127 / (255 - t)
                };
                let value = if options.invert { 255 - value } else { value };
                self.chars[(value as usize * (self.chars.len() - 1) + 127) / 255]
            });
            self.threshold = Some(t as u8);
        }
        cells.clear();
        let samples_per_cell = match options.mode {
            Mode::Ascii => 1,
            Mode::Braille => 8,
            Mode::Halfblock => 2,
        };
        cells.reserve(packed.len() / samples_per_cell);
        match options.dither {
            Dither::None => self.map::<false>(packed, width as usize, threshold, options, cells),
            Dither::Bayer => self.map::<true>(packed, width as usize, threshold, options, cells),
        }
    }
    fn map<const DITHER: bool>(
        &self,
        packed: &[u32],
        width: usize,
        threshold: u8,
        options: &Options,
        cells: &mut Vec<Cell>,
    ) {
        match options.mode {
            Mode::Ascii => {
                let step = 255 / (self.chars.len() - 1) as i32;
                for (y, row) in packed.chunks_exact(width).enumerate() {
                    for (x, &p) in row.iter().enumerate() {
                        let mut l = (p >> 24) as usize;
                        if DITHER && l > 0 && l < 255 {
                            l = (l as i32 + (2 * BAYER[y % 4][x % 4] - 15) * step / 32)
                                .clamp(0, 255) as usize;
                        }
                        cells.push(Cell {
                            glyph: self.ascii[l],
                            rgb: rgb(p),
                            background: None,
                            monochrome: None,
                        });
                    }
                }
            }
            Mode::Braille => {
                let stride = width * 2;
                // A group contains four sample rows for one output row.
                for (y, rows) in packed.chunks_exact(stride * 4).enumerate() {
                    for x in 0..width {
                        let mut sums = [0u32; 3];
                        let mut bits = 0;
                        for (dx, dy, bit) in [
                            (0, 0, 0),
                            (1, 0, 3),
                            (0, 1, 1),
                            (1, 1, 4),
                            (0, 2, 2),
                            (1, 2, 5),
                            (0, 3, 6),
                            (1, 3, 7),
                        ] {
                            let p = rows[dy * stride + x * 2 + dx];
                            sums[0] += p & 255;
                            sums[1] += p >> 8 & 255;
                            sums[2] += p >> 16 & 255;
                            if lit::<DITHER>(p, x * 2 + dx, y * 4 + dy, threshold, options.invert) {
                                bits |= 1 << bit;
                            }
                        }
                        cells.push(Cell {
                            glyph: char::from_u32(0x2800 + bits).unwrap(),
                            rgb: sums.map(|v| (v / 8) as u8),
                            background: None,
                            monochrome: None,
                        });
                    }
                }
            }
            Mode::Halfblock => {
                for (y, rows) in packed.chunks_exact(width * 2).enumerate() {
                    let (top, bottom) = rows.split_at(width);
                    for (x, (&a, &b)) in top.iter().zip(bottom).enumerate() {
                        let bits =
                            usize::from(lit::<DITHER>(a, x, y * 2, threshold, options.invert))
                                | usize::from(lit::<DITHER>(
                                    b,
                                    x,
                                    y * 2 + 1,
                                    threshold,
                                    options.invert,
                                )) << 1;
                        let (a, b) = (rgb(a), rgb(b));
                        cells.push(if options.halfblock_color {
                            Cell {
                                glyph: '▀',
                                rgb: a,
                                background: Some(b),
                                monochrome: Some(HALF[bits]),
                            }
                        } else {
                            Cell {
                                glyph: HALF[bits],
                                rgb: std::array::from_fn(|c| {
                                    ((u16::from(a[c]) + u16::from(b[c])) / 2) as u8
                                }),
                                background: None,
                                monochrome: None,
                            }
                        });
                    }
                }
            }
        }
    }
}
#[inline]
fn rgb(p: u32) -> [u8; 3] {
    [p as u8, (p >> 8) as u8, (p >> 16) as u8]
}
#[inline]
fn lit<const DITHER: bool>(p: u32, x: usize, y: usize, threshold: u8, invert: bool) -> bool {
    let l = p >> 24;
    let on = if DITHER {
        let offset = (2 * BAYER[y % 4][x % 4] - 15) * 4;
        l as i32 > (i32::from(threshold) + offset).clamp(0, 254) && threshold < 255
    } else {
        l > u32::from(threshold)
    };
    on ^ invert
}
