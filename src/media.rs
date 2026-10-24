//! Local media decoding. GIF is incremental; video uses a bounded FFmpeg pipe.
use anyhow::{Context, Result, bail};
use image::{AnimationDecoder, RgbaImage};
use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Read, Seek},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
};
pub const MAX_PIXELS: u64 = 33_554_432;
const BMP_BUFFER_BYTES: u64 = MAX_PIXELS * 4 + 65_536;
pub struct Frame {
    pub image: RgbaImage,
    pub pts: f64,
    pub duration: f64,
}
pub enum Source {
    Still(Option<RgbaImage>),
    Gif {
        frames: image::Frames<'static>,
        pts: f64,
    },
    Video(Video),
}
fn check_size(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        bail!("input exceeds 33,554,432 pixels or is empty");
    }
    Ok(())
}
fn decode_bmp(reader: impl BufRead + Seek) -> Result<RgbaImage> {
    use image::ImageDecoder;
    let mut decoder = image::codecs::bmp::BmpDecoder::new(reader)?;
    let (w, h) = decoder.dimensions();
    check_size(w, h)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 8);
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    Ok(image::DynamicImage::from_decoder(decoder)?.into_rgba8())
}
impl Source {
    pub fn open(path: &Path) -> Result<Self> {
        if !path.is_file() {
            bail!("input must be a local regular file: {}", path.display());
        }
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        if reader.format() == Some(image::ImageFormat::Bmp) {
            let mut input = reader.into_inner();
            let encoded_size = input.get_ref().metadata()?.len();
            if encoded_size <= BMP_BUFFER_BYTES {
                let (w, h) = image::ImageReader::open(path)?
                    .with_guessed_format()?
                    .into_dimensions()?;
                check_size(w, h)?;
                // Bound the cache to one maximum-size RGBA frame plus BMP headers.
                let mut bytes = vec![0; encoded_size as usize];
                input.read_exact(&mut bytes)?;
                return Ok(Self::Still(Some(decode_bmp(Cursor::new(bytes))?)));
            }
            reader = image::ImageReader::with_format(input, image::ImageFormat::Bmp);
        }
        if reader.format() == Some(image::ImageFormat::Gif) {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(BufReader::new(File::open(path)?))?;
            use image::ImageDecoder;
            let (w, h) = decoder.dimensions();
            check_size(w, h)?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(MAX_PIXELS * 8);
            decoder.set_limits(limits)?;
            return Ok(Self::Gif {
                frames: decoder.into_frames(),
                pts: 0.,
            });
        }
        if reader.format().is_some() {
            let (w, h) = image::ImageReader::open(path)?
                .with_guessed_format()?
                .into_dimensions()?;
            check_size(w, h)?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(MAX_PIXELS * 8);
            reader.limits(limits);
            return Ok(Self::Still(Some(reader.decode()?.into_rgba8())));
        }
        Ok(Self::Video(Video::open(path)?))
    }
    /// Return a consumed video's image buffer for the next frame. Keeps at most one
    /// buffer; still/GIF sources and mismatched dimensions simply drop the image.
    /// Call only after finishing all processing that needs the owned image.
    pub fn recycle(&mut self, image: RgbaImage) {
        if let Self::Video(video) = self
            && image.dimensions() == (video.width, video.height)
        {
            let mut bytes = image.into_raw();
            // ImageBuffer permits trailing storage; never read it as another frame.
            bytes.truncate(video.width as usize * video.height as usize * 4);
            video.reusable = Some(bytes);
        }
    }
}
impl Iterator for Source {
    type Item = Result<Frame>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Still(image) => image.take().map(|image| {
                Ok(Frame {
                    image,
                    pts: 0.,
                    duration: 0.,
                })
            }),
            Self::Gif { frames, pts } => frames.next().map(|frame| {
                let frame = frame?;
                let (n, d) = frame.delay().numer_denom_ms();
                let duration = (n as f64 / d as f64 / 1000.).max(0.001);
                let time = *pts;
                *pts += duration;
                Ok(Frame {
                    image: frame.into_buffer(),
                    pts: time,
                    duration,
                })
            }),
            Self::Video(video) => video.next(),
        }
    }
}
pub struct Video {
    child: Child,
    width: u32,
    height: u32,
    duration: f64,
    times: Option<Receiver<f64>>,
    reader: Option<JoinHandle<()>>,
    done: bool,
    reusable: Option<Vec<u8>>,
}
impl Video {
    fn open(path: &Path) -> Result<Self> {
        let result = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height,avg_frame_rate",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .context("video requires ffprobe and ffmpeg in PATH")?;
        if !result.status.success() {
            bail!("ffprobe could not decode input");
        }
        let info: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        let stream = &info["streams"][0];
        let width = stream["width"].as_u64().context("missing video width")?;
        let height = stream["height"].as_u64().context("missing video height")?;
        if width > u32::MAX as u64 || height > u32::MAX as u64 {
            bail!("video dimensions overflow");
        }
        let (width, height) = (width as u32, height as u32);
        check_size(width, height)?;
        let rate = stream["avg_frame_rate"].as_str().unwrap_or("25/1");
        let (a, b) = rate.split_once('/').unwrap_or(("25", "1"));
        let fps = a.parse::<f64>().unwrap_or(25.) / b.parse::<f64>().unwrap_or(1.);
        let duration = if fps.is_finite() && fps > 0. {
            1. / fps
        } else {
            0.04
        };
        let mut child = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "info", "-noautorotate", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-vf",
                "showinfo=checksum=0",
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("cannot start ffmpeg")?;
        let stderr = child.stderr.take().context("missing stderr")?;
        let (tx, rx) = mpsc::sync_channel(4);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let time = line
                    .split(" pts_time:")
                    .nth(1)
                    .and_then(|after| after.split_whitespace().next())
                    .and_then(|s| s.parse::<f64>().ok());
                if let Some(time) = time
                    && tx.send(time).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            width,
            height,
            duration,
            times: Some(rx),
            reader: Some(reader),
            done: false,
            reusable: None,
        })
    }
}
impl Iterator for Video {
    type Item = Result<Frame>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let size = (self.width as usize) * (self.height as usize) * 4;
        let mut bytes = self.reusable.take().unwrap_or_else(|| vec![0u8; size]);
        let stdout = self.child.stdout.as_mut()?;
        // Distinguish clean EOF from a truncated final frame.
        let first = match stdout.read(&mut bytes[..1]) {
            Ok(n) => n,
            Err(e) => {
                self.done = true;
                return Some(Err(e.into()));
            }
        };
        if first == 0 {
            self.done = true;
            return match self.child.wait() {
                Ok(s) if s.success() => None,
                Ok(s) => Some(Err(anyhow::anyhow!("ffmpeg failed: {s}"))),
                Err(e) => Some(Err(e.into())),
            };
        }
        if let Err(e) = stdout.read_exact(&mut bytes[1..]) {
            self.done = true;
            return Some(Err(e).context("truncated FFmpeg frame"));
        }
        let pts = match self.times.as_ref()?.recv() {
            Ok(t) if t.is_finite() => t,
            _ => {
                self.done = true;
                return Some(Err(anyhow::anyhow!("missing FFmpeg frame timestamp")));
            }
        };
        Some(Ok(Frame {
            image: RgbaImage::from_raw(self.width, self.height, bytes).expect("checked size"),
            pts,
            duration: self.duration,
        }))
    }
}
impl Drop for Video {
    fn drop(&mut self) {
        self.times.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(t) = self.reader.take() {
            let _ = t.join();
        }
    }
}
