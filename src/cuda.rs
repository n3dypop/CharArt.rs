//! NVIDIA cuTile integer pixel pipeline. Nearest resize and glyph layout stay on CPU.
use anyhow::Result;
use cuda_core::{Device, Stream};
use cutile::prelude::*;
use image::RgbaImage;
use std::sync::Arc;
#[cutile::module]
mod pixels {
    use cutile::core::*;
    #[cutile::entry()]
    pub fn transform<const B: i32>(
        out: &mut Tensor<u32, { [B] }>,
        input: &Tensor<u32, { [-1] }>,
        red: u32,
        green: u32,
        blue: u32,
        levels: u32,
    ) {
        let k1 = 1u32.broadcast(out.shape());
        let k2 = 2u32.broadcast(out.shape());
        let k29 = 29u32.broadcast(out.shape());
        let k77 = 77u32.broadcast(out.shape());
        let k127 = 127u32.broadcast(out.shape());
        let k128 = 128u32.broadcast(out.shape());
        let k150 = 150u32.broadcast(out.shape());
        let k255 = 255u32.broadcast(out.shape());
        let k256 = 256u32.broadcast(out.shape());
        let k65536 = 65536u32.broadcast(out.shape());
        let k16777216 = 16777216u32.broadcast(out.shape());
        let p = input.load_like(out);
        let a = p / k16777216;
        let br = red.broadcast(out.shape());
        let bg = green.broadcast(out.shape());
        let bb = blue.broadcast(out.shape());
        let q = levels.broadcast(out.shape());
        let r = ((p % k256) * a + br * (k255 - a) + k127) / k255;
        let g = ((p / k256 % k256) * a + bg * (k255 - a) + k127) / k255;
        let b = ((p / k65536 % k256) * a + bb * (k255 - a) + k127) / k255;
        let l = (r * k77 + g * k150 + b * k29 + k128) / k256;
        let r = ((r * (q - k1) + k127) / k255 * k255 + (q - k1) / k2) / (q - k1);
        let g = ((g * (q - k1) + k127) / k255 * k255 + (q - k1) / k2) / (q - k1);
        let b = ((b * (q - k1) + k127) / k255 * k255 + (q - k1) / k2) / (q - k1);
        out.store(r + g * k256 + b * k65536 + l * k16777216);
    }
}
pub struct Gpu {
    device: Arc<Device>,
    stream: Arc<Stream>,
    output: Option<Tensor<u32>>,
    size: usize,
}
impl Gpu {
    pub fn new() -> Result<Self> {
        let device = Device::new(0)?;
        let stream = device.new_stream()?;
        Ok(Self {
            device,
            stream,
            output: None,
            size: 0,
        })
    }
    pub fn process(&mut self, image: &RgbaImage, bg: [u8; 3], q: u32) -> Result<(Vec<u32>, f32)> {
        let data: Arc<Vec<u32>> =
            Arc::new(image.pixels().map(|p| u32::from_le_bytes(p.0)).collect());
        let input: Arc<Tensor<u32>> = api::copy_host_vec_to_device(&data)
            .sync_on(&self.stream)?
            .into();
        let fresh = self.output.is_none() || self.size != data.len();
        let mut output = if fresh {
            api::zeros::<u32>(&[data.len()]).sync_on(&self.stream)?
        } else {
            self.output.take().unwrap()
        };
        if fresh {
            // Warm JIT before the timing events; cold cost remains in processing_ms.
            let (p, _, _, _, _, _) = pixels::transform(
                output.partition([128]),
                input.clone(),
                bg[0] as u32,
                bg[1] as u32,
                bg[2] as u32,
                q,
            )
            .sync_on(&self.stream)?;
            output = p.unpartition();
        }
        let start = self.device.new_event()?;
        let end = self.device.new_event()?;
        start.record(&self.stream)?;
        let (p, _, _, _, _, _) = pixels::transform(
            output.partition([128]),
            input,
            bg[0] as u32,
            bg[1] as u32,
            bg[2] as u32,
            q,
        )
        .sync_on(&self.stream)?;
        end.record(&self.stream)?;
        end.synchronize()?;
        let ms = start.elapsed_time(&end)?;
        let output = Arc::new(p.unpartition());
        let host = api::copy_device_to_host_vec(&output).sync_on(&self.stream)?;
        self.output =
            Some(Arc::try_unwrap(output).map_err(|_| {
                anyhow::anyhow!("CUDA output still borrowed after synchronization")
            })?);
        self.size = data.len();
        Ok((host, ms))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_compiles_to_tile_ir_without_gpu() {
        std::thread::Builder::new()
            .stack_size(8_000_000)
            .spawn(|| {
                let artifacts = cutile::compile_api::KernelCompiler::new(
                    pixels::__module_ast_self,
                    "pixels",
                    "transform",
                )
                .generics(vec!["128".into()])
                .strides(&[("out", &[1]), ("input", &[1])])
                .target("sm_80")
                .bytecode_version(cutile_ir::bytecode::BytecodeVersion::V13_2)
                .compile()
                .unwrap();
                assert!(artifacts.ir_text().contains("entry"));
                assert!(!artifacts.bytecode().unwrap().is_empty());
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    #[ignore = "requires supported NVIDIA GPU, driver and CUDA toolkit; run explicitly"]
    fn gpu_matches_cpu_exactly() {
        let mut gpu = Gpu::new().unwrap();
        for n in [1, 127, 128, 129, 4096] {
            for q in [2, 3, 16, 256] {
                let image = RgbaImage::from_fn(n, 1, |x, _| {
                    image::Rgba([x as u8, (x * 71) as u8, (x * 13) as u8, (x * 17) as u8])
                });
                for bg in [[0, 0, 0], [255, 120, 31]] {
                    let (actual, _) = gpu.process(&image, bg, q).unwrap();
                    assert_eq!(actual, crate::cpu_pixels(&image, bg, q).unwrap());
                }
            }
        }
        // Compare final glyphs/colors, including Otsu and partial tiles.
        for mode in [
            crate::Mode::Ascii,
            crate::Mode::Braille,
            crate::Mode::Halfblock,
        ] {
            let image = RgbaImage::from_fn(137, 29, |x, y| {
                image::Rgba([x as u8, y as u8, (x * y) as u8, 190])
            });
            let o = crate::Options {
                width: Some(37),
                mode,
                backend: crate::Backend::Cpu,
                dither: if mode == crate::Mode::Halfblock {
                    crate::Dither::None
                } else {
                    crate::Dither::Bayer
                },
                tone: crate::Tone {
                    gamma: 1.25,
                    auto_contrast: true,
                    ..Default::default()
                },
                halfblock_color: true,
                ..Default::default()
            };
            let cpu = crate::Converter::new(o.clone())
                .unwrap()
                .convert(&image)
                .unwrap();
            let cuda = crate::Converter::new(crate::Options {
                backend: crate::Backend::Cuda,
                ..o
            })
            .unwrap()
            .convert(&image)
            .unwrap();
            assert_eq!(cpu.cells, cuda.cells);
        }
    }
}
