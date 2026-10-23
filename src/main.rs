use anyhow::{Context, Result, bail};
use charart::{
    Art, Backend, Converter, Dither, Mode, Options,
    media::Source,
    output::{self, Format},
    tuning,
};
use clap::Parser;
use std::{
    fs::File,
    io::{self, IsTerminal, Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Parser)]
#[command(version, about = "Stream images, GIFs and videos into character art")]
struct Args {
    input: PathBuf,
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "auto")]
    backend: Backend,
    #[arg(long, value_enum)]
    mode: Option<Mode>,
    #[arg(long, value_enum)]
    format: Option<Format>,
    #[arg(long)]
    color: bool,
    #[arg(short, long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long, default_value_t = 1.)]
    scale: f64,
    #[arg(long, default_value_t = 0.5)]
    cell_aspect: f64,
    #[arg(long, default_value = " .:-=+*#%@")]
    charset: String,
    #[arg(long)]
    invert: bool,
    /// 0..255, or auto for Otsu.
    #[arg(long, default_value = "auto")]
    threshold: String,
    /// Levels per RGB channel, 2..256 (not total palette size).
    #[arg(long, default_value_t = 256)]
    quantize: u32,
    #[arg(long, default_value = "000000")]
    background: String,
    #[arg(long)]
    play: bool,
    #[arg(long = "loop", requires = "play")]
    looping: bool,
    #[arg(long, default_value_t = 1.)]
    speed: f64,
    /// Stop after this many frames per pass.
    #[arg(long)]
    frames: Option<u64>,
    /// Write machine-readable run timing to stderr.
    #[arg(long)]
    timings: bool,
    /// Ordered dithering for ASCII/Braille and monochrome halfblocks.
    #[arg(long, value_enum)]
    dither: Option<Dither>,
    #[arg(long, allow_hyphen_values = true)]
    brightness: Option<f32>,
    #[arg(long)]
    contrast: Option<f32>,
    #[arg(long)]
    gamma: Option<f32>,
    /// Freeze percentile contrast levels from the first frame.
    #[arg(long,num_args=0..=1,default_missing_value="true",require_equals=true)]
    auto_contrast: Option<bool>,
    /// Analyze first frame and apply explainable recommendations.
    #[arg(long, conflicts_with = "profile")]
    auto: bool,
    /// Export a reusable tuning profile instead of art (JSON).
    #[arg(long,conflicts_with_all=["play","profile"])]
    analyze: bool,
    /// Load a profile; explicit CLI settings take precedence.
    #[arg(long)]
    profile: Option<PathBuf>,
}
struct Terminal;
impl Terminal {
    fn enter() -> Result<Self> {
        let guard = Self;
        crossterm::execute!(
            io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            io::stdout(),
            crossterm::style::ResetColor,
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen
        );
    }
}
fn background(s: &str) -> Result<[u8; 3]> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.is_ascii() {
        bail!("background must be six hex digits");
    }
    Ok([
        u8::from_str_radix(&s[0..2], 16)?,
        u8::from_str_radix(&s[2..4], 16)?,
        u8::from_str_radix(&s[4..6], 16)?,
    ])
}
fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("charart: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
fn run(a: Args) -> Result<()> {
    let total = Instant::now();
    if !a.speed.is_finite() || !(0.01..=100.).contains(&a.speed) {
        bail!("speed must be finite and in 0.01..100");
    }
    if a.frames == Some(0) {
        bail!("frames must be positive");
    }
    if a.play && !io::stdout().is_terminal() {
        bail!("--play requires a terminal on stdout");
    }
    if let Some(out) = &a.output
        && out.exists()
        && out.canonicalize()? == a.input.canonicalize()?
    {
        bail!("output must differ from input");
    }
    let threshold = if a.threshold == "auto" {
        None
    } else {
        Some(
            a.threshold
                .parse::<u8>()
                .context("threshold must be auto or 0..255")?,
        )
    };
    let width = a.width.or_else(|| {
        if a.height.is_some() {
            None
        } else {
            Some(
                crossterm::terminal::size()
                    .ok()
                    .filter(|s| s.0 > 0)
                    .map(|s| u32::from(s.0))
                    .unwrap_or(80)
                    .min(4096),
            )
        }
    });
    let format = a.format.unwrap_or_else(|| {
        match a
            .output
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|x| x.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("html") => Format::Html,
            Some("lua") => Format::Lua,
            Some("json" | "jsonl") => Format::Json,
            Some("ansi") => Format::Ansi,
            _ => {
                if a.color {
                    Format::Ansi
                } else {
                    Format::Text
                }
            }
        }
    });
    let mut source = Source::open(&a.input)?;
    let animated = !matches!(source, Source::Still(_));
    let first = if a.auto || a.analyze {
        Some(
            source
                .next()
                .transpose()?
                .context("input contains no decodable frames")?,
        )
    } else {
        None
    };
    let mut profile = if let Some(frame) = &first {
        Some(tuning::analyze(
            &frame.image,
            background(&a.background)?,
            a.color && !a.invert && a.dither != Some(Dither::Bayer),
        )?)
    } else if let Some(path) = &a.profile {
        let mut json = String::new();
        File::open(path)?
            .take(1_048_577)
            .read_to_string(&mut json)?;
        if json.len() > 1_048_576 {
            bail!("profile exceeds 1 MiB");
        }
        Some(tuning::Profile::from_json(&json)?)
    } else {
        None
    };
    let suggested = profile.as_ref().map(|p| &p.settings);
    let mut tone = suggested.map(|s| s.tone).unwrap_or_default();
    if let Some(v) = a.brightness {
        tone.brightness = v;
    }
    if let Some(v) = a.contrast {
        tone.contrast = v;
    }
    if let Some(v) = a.gamma {
        tone.gamma = v;
    }
    if let Some(v) = a.auto_contrast {
        tone.auto_contrast = v;
    }
    let mode = a
        .mode
        .or_else(|| suggested.map(|s| s.mode))
        .unwrap_or(Mode::Ascii);
    let halfblock_color = a.color && (a.play || format != Format::Text);
    let suggested_dither = if mode == Mode::Halfblock && halfblock_color {
        Dither::None
    } else {
        suggested.map(|s| s.dither).unwrap_or_default()
    };
    let options = Options {
        width,
        height: a.height,
        scale: a.scale,
        cell_aspect: a.cell_aspect,
        charset: a.charset,
        invert: a.invert,
        threshold,
        quantize: a.quantize,
        background: background(&a.background)?,
        mode,
        backend: a.backend,
        dither: a.dither.unwrap_or(suggested_dither),
        tone,
        halfblock_color,
    };
    options.validate()?;
    if let Some(p) = &mut profile {
        p.settings = tuning::Settings {
            mode: options.mode,
            dither: options.dither,
            tone: options.tone,
        };
        if a.mode.is_some()
            || a.dither.is_some()
            || a.brightness.is_some()
            || a.gamma.is_some()
            || a.contrast.is_some()
            || a.auto_contrast.is_some()
        {
            p.reasons
                .push("Explicit CLI values override recommendations".into());
        }
    }
    if a.analyze {
        let text =
            serde_json::to_string_pretty(profile.as_ref().expect("analyzed first frame"))? + "\n";
        if let Some(path) = &a.output {
            std::fs::write(path, text)?;
        } else {
            io::stdout().write_all(text.as_bytes())?;
        }
        return Ok(());
    }
    if let Some(p) = &profile {
        eprintln!(
            "charart: tuning settings {}",
            serde_json::to_string(&p.settings)?
        );
    }
    if matches!(format, Format::Lua | Format::Html) && animated && a.frames != Some(1) {
        bail!(
            "HTML/Lua export requires --frames 1 for animated inputs; use JSON/text/ANSI for a stream"
        );
    }
    let mut converter = Converter::new(options)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed))?;
    let _terminal = if a.play {
        Some(Terminal::enter()?)
    } else {
        None
    };
    let mut saved: Option<Box<dyn Write>> = a
        .output
        .as_ref()
        .map(|p| File::create(p).map(|f| Box::new(io::BufWriter::new(f)) as Box<dyn Write>))
        .transpose()?;
    let mut stdout = io::stdout().lock();
    let mut art: Option<Art> = None;
    let mut text = String::new();
    let mut json = Vec::new();
    let mut playback = String::new();
    let mut first_pass = true;
    let mut source = Some(source);
    let mut pending = first;
    let mut count = 0u64;
    let mut processing_ms = 0.;
    let mut kernel_ms = 0.;
    let mut notified = None;
    loop {
        let mut stream = match source.take() {
            Some(s) => s,
            None => Source::open(&a.input)?,
        };
        let mut pass_count = 0;
        let mut origin = None;
        let wall = Instant::now();
        let mut end = 0.;
        while !stop.load(Ordering::Relaxed) && a.frames.is_none_or(|n| pass_count < n) {
            let next = pending.take().map(Ok).or_else(|| stream.next());
            let Some(frame) = next else {
                break;
            };
            let frame = frame?;
            let base = *origin.get_or_insert(frame.pts);
            let at = ((frame.pts - base) / a.speed).clamp(0., 86400. * 365.);
            if a.play {
                wait_until(wall, at, &stop);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            if let Some(art) = &mut art {
                converter.convert_into(&frame.image, art)?;
            } else {
                art = Some(converter.convert(&frame.image)?);
            }
            let art = art.as_ref().expect("converted frame");
            if let Some(reason) = &art.diagnostics.fallback_reason
                && notified.as_ref() != Some(reason)
            {
                eprintln!("charart: auto selected CPU: {reason}");
                notified = Some(reason.clone());
            }
            processing_ms += art.diagnostics.processing_ms;
            kernel_ms += art.diagnostics.kernel_ms.unwrap_or(0.) as f64;
            let needs_saved = first_pass && (saved.is_some() || !a.play);
            let bytes = if needs_saved {
                if format == Format::Json {
                    json.clear();
                    serde_json::to_writer(
                        &mut json,
                        &FrameExport {
                            pts_seconds: frame.pts,
                            duration_seconds: frame.duration,
                            art,
                        },
                    )?;
                    json.push(b'\n');
                    json.as_slice()
                } else {
                    output::render_into(art, format, a.color, &mut text);
                    text.as_bytes()
                }
            } else {
                &[]
            };
            if first_pass {
                if let Some(out) = saved.as_mut() {
                    if pass_count > 0 && matches!(format, Format::Text | Format::Ansi) {
                        out.write_all(b"\x0c\n")?;
                    }
                    out.write_all(bytes)?;
                } else if !a.play {
                    if pass_count > 0 && matches!(format, Format::Text | Format::Ansi) {
                        stdout.write_all(b"\x0c\n")?;
                    }
                    stdout.write_all(bytes)?;
                }
            }
            if a.play {
                crossterm::execute!(
                    stdout,
                    crossterm::cursor::MoveTo(0, 0),
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
                )?;
                if format == Format::Ansi && needs_saved {
                    stdout.write_all(bytes)?;
                } else {
                    output::render_into(art, Format::Ansi, a.color, &mut playback);
                    stdout.write_all(playback.as_bytes())?;
                }
                stdout.flush()?;
            }
            count += 1;
            pass_count += 1;
            end = at + frame.duration / a.speed;
            stream.recycle(frame.image);
        }
        if a.play {
            wait_until(wall, end, &stop);
        }
        if pass_count == 0 && !stop.load(Ordering::Relaxed) {
            bail!("input contains no decodable frames");
        }
        if !a.looping || stop.load(Ordering::Relaxed) {
            break;
        }
        first_pass = false;
    }
    if let Some(out) = saved.as_mut() {
        out.flush()?;
    }
    stdout.flush()?;
    if a.timings {
        eprintln!(
            "{}",
            serde_json::json!({"frames":count,"total_ms":total.elapsed().as_secs_f64()*1000.,"processing_ms":processing_ms,"kernel_ms_sum":kernel_ms,"total_includes_playback_waits":a.play})
        );
    }
    Ok(())
}
#[derive(serde::Serialize)]
struct FrameExport<'a> {
    pts_seconds: f64,
    duration_seconds: f64,
    art: &'a Art,
}
fn wait_until(start: Instant, seconds: f64, stop: &AtomicBool) {
    let target = Duration::from_secs_f64(seconds);
    while !stop.load(Ordering::Relaxed) {
        let Some(left) = target.checked_sub(start.elapsed()) else {
            break;
        };
        std::thread::sleep(left.min(Duration::from_millis(10)));
    }
}
