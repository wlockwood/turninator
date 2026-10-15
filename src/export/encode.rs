use super::jobs::ExportResult;
use crate::{
    Result,
    render::worker::{ToolPaths, run_owned},
};
use std::{
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub struct Scratch(pub PathBuf);
impl Scratch {
    pub fn new(parent: &Path) -> Result<Self> {
        fs::create_dir_all(parent)?;
        let name = format!(
            ".turninator-{}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = parent.join(name);
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn validate_frames(directory: &Path, count: u32, size: u32) -> Result<Vec<PathBuf>> {
    validate_frames_dimensions(directory, count, size, size)
}
pub fn validate_frames_dimensions(
    directory: &Path,
    count: u32,
    width: u32,
    height: u32,
) -> Result<Vec<PathBuf>> {
    if !(1..=10000).contains(&count)
        || width < 16
        || height < 16
        || width as u64 * height as u64 > 64_000_000
    {
        return Err("invalid frame count or size".into());
    }
    let mut result = Vec::new();
    for i in 0..count {
        let path = directory.join(format!("frame_{i:04}.png"));
        validate_png(&path, width, height)?;
        result.push(path);
    }
    Ok(result)
}
pub fn validate_png(path: &Path, width: u32, height: u32) -> Result<()> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    if reader.format() != Some(image::ImageFormat::Png) {
        return Err("rendered frame is not PNG".into());
    }
    let decoded = reader.decode()?;
    if decoded.width() != width || decoded.height() != height {
        return Err(format!("wrong dimensions in {}", path.display()).into());
    }
    Ok(())
}

pub fn publish(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    fs::rename(source, destination)?;
    Ok(())
}

fn result(path: &Path, width: u32, height: u32, count: u32, duration: u64) -> Result<ExportResult> {
    Ok(ExportResult {
        path: path.to_owned(),
        width,
        height,
        bytes: fs::metadata(path)?.len(),
        frame_count: count,
        duration_ms: duration,
    })
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err("cancelled".into())
    } else {
        Ok(())
    }
}
fn arguments(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

pub fn publish_png(
    source: &Path,
    destination: &Path,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    validate_png(source, width, height)?;
    cancelled(cancel)?;
    let scratch = Scratch::new(destination.parent().ok_or("destination has no parent")?)?;
    let temp = scratch.0.join("output.png");
    fs::copy(source, &temp)?;
    validate_png(&temp, width, height)?;
    cancelled(cancel)?;
    publish(&temp, destination)?;
    result(destination, width, height, 1, 0)
}

pub fn inspect_gif(path: &Path, size: u32, count: u32) -> Result<()> {
    inspect_gif_dimensions(path, size, size, count)
}
pub fn inspect_gif_dimensions(path: &Path, width: u32, height: u32, count: u32) -> Result<()> {
    inspect_gif_dimensions_at_rate(path, width, height, count, 20)
}

fn validate_frame_rate(frame_rate: u32) -> Result<()> {
    if !(1..=100).contains(&frame_rate) {
        return Err("frame rate must be between 1 and 100 FPS".into());
    }
    Ok(())
}

/// Quantize absolute frame boundaries, avoiding cumulative GIF timing drift.
pub fn gif_frame_delays(count: u32, frame_rate: u32) -> Result<Vec<u16>> {
    validate_frame_rate(frame_rate)?;
    if !(1..=10_000).contains(&count) {
        return Err("frame count must be between 1 and 10000".into());
    }
    let rate = u64::from(frame_rate);
    let boundary = |index: u32| (u64::from(index) * 100 + rate / 2) / rate;
    Ok((0..count)
        .map(|index| (boundary(index + 1) - boundary(index)) as u16)
        .collect())
}

pub fn inspect_gif_dimensions_at_rate(
    path: &Path,
    width: u32,
    height: u32,
    count: u32,
    frame_rate: u32,
) -> Result<()> {
    let delays = gif_frame_delays(count, frame_rate)?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(BufReader::new(fs::File::open(path)?))?;
    if decoder.width() as u32 != width || decoder.height() as u32 != height {
        return Err("encoded GIF dimensions differ".into());
    }
    let mut found = 0;
    while let Some(frame) = decoder.read_next_frame()? {
        if delays.get(found).copied() != Some(frame.delay) {
            return Err("GIF frame delay differs from requested frame rate".into());
        }
        found += 1;
    }
    if found != count as usize || decoder.repeat() != gif::Repeat::Infinite {
        return Err("GIF frame count or infinite loop metadata differs".into());
    }
    Ok(())
}

const GIF_DITHER: &str = "sierra2_4a";

pub fn gif_encoding_recipe() -> serde_json::Value {
    gif_encoding_recipe_at_rate(20)
}

pub fn gif_encoding_recipe_at_rate(frame_rate: u32) -> serde_json::Value {
    let constant_delay = if (1..=100).contains(&frame_rate) && 100_u32.is_multiple_of(frame_rate) {
        Some(100 / frame_rate)
    } else {
        None
    };
    serde_json::json!({"palette":format!("full,opaque,{GIF_DITHER}"),"fps":frame_rate,"final_delay_cs":constant_delay,"delay_policy":"round_absolute_centisecond_boundaries","loop":0})
}

pub fn encode_gif(
    frames: &Path,
    destination: &Path,
    tools: &ToolPaths,
    count: u32,
    size: u32,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    encode_gif_dimensions(frames, destination, tools, count, size, size, cancel)
}
pub fn encode_gif_dimensions(
    frames: &Path,
    destination: &Path,
    tools: &ToolPaths,
    count: u32,
    width: u32,
    height: u32,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    encode_gif_dimensions_at_rate(frames, destination, tools, count, width, height, 20, cancel)
}

#[allow(clippy::too_many_arguments)]
pub fn encode_gif_dimensions_at_rate(
    frames: &Path,
    destination: &Path,
    tools: &ToolPaths,
    count: u32,
    width: u32,
    height: u32,
    frame_rate: u32,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    let delays = gif_frame_delays(count, frame_rate)?;
    let rate = frame_rate.to_string();
    let final_delay = delays.last().ok_or("missing GIF frame delay")?.to_string();
    validate_frames_dimensions(frames, count, width, height)?;
    cancelled(cancel)?;
    let scratch = Scratch::new(destination.parent().ok_or("destination has no parent")?)?;
    let palette = scratch.0.join("palette.png");
    let encoded = scratch.0.join("output.gif");
    let common = arguments(&[
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
    ]);
    let mut args = common.clone();
    args.extend(arguments(&[
        "-framerate",
        &rate,
        "-start_number",
        "0",
        "-i",
        "frame_%04d.png",
        "-vf",
    ]));
    args.push(format!(
        "trim=end_frame={count},palettegen=stats_mode=full:reserve_transparent=0"
    ));
    args.extend(arguments(&["-frames:v", "1", "-update", "1"]));
    args.push(palette.to_string_lossy().into_owned());
    run_owned(
        &tools.ffmpeg,
        &args,
        Some(frames),
        cancel,
        Duration::from_secs(600),
    )?;
    let mut args = common;
    args.extend(arguments(&[
        "-framerate",
        &rate,
        "-start_number",
        "0",
        "-i",
        "frame_%04d.png",
        "-f",
        "image2",
        "-pattern_type",
        "none",
        "-i",
    ]));
    args.push(palette.to_string_lossy().into_owned());
    args.extend(arguments(&["-lavfi"]));
    args.push(format!("paletteuse=dither={GIF_DITHER}"));
    args.extend(arguments(&["-frames:v"]));
    args.push(count.to_string());
    args.extend(arguments(&[
        "-r",
        &rate,
        "-loop",
        "0",
        "-final_delay",
        &final_delay,
    ]));
    args.push(encoded.to_string_lossy().into_owned());
    run_owned(
        &tools.ffmpeg,
        &args,
        Some(frames),
        cancel,
        Duration::from_secs(600),
    )?;
    inspect_gif_dimensions_at_rate(&encoded, width, height, count, frame_rate)?;
    cancelled(cancel)?;
    publish(&encoded, destination)?;
    let duration_ms = delays.iter().map(|&delay| u64::from(delay) * 10).sum();
    result(destination, width, height, count, duration_ms)
}

pub fn encode_mp4(
    frames: &Path,
    destination: &Path,
    tools: &ToolPaths,
    count: u32,
    size: u32,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    encode_mp4_with_codec(
        frames,
        destination,
        tools,
        count,
        size,
        size,
        super::jobs::VideoCodec::H264,
        cancel,
    )
}
pub fn video_encoder(
    tools: &ToolPaths,
    codec: super::jobs::VideoCodec,
    cancel: &AtomicBool,
) -> Result<String> {
    let output = run_owned(
        &tools.ffmpeg,
        &arguments(&["-hide_banner", "-encoders"]),
        None,
        cancel,
        Duration::from_secs(30),
    )?;
    let text = String::from_utf8_lossy(&output);
    let names = text
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect::<Vec<_>>();
    let candidates = match codec {
        super::jobs::VideoCodec::H264 => &["libx264"][..],
        super::jobs::VideoCodec::Av1 => &["av1_nvenc", "libaom-av1", "libsvtav1"][..],
    };
    candidates.iter().find(|name|names.contains(name)).map(|s|s.to_string()).ok_or_else(||format!("FFmpeg has no encoder for {codec:?}; choose another codec or configure a build with AV1/libx264 support").into())
}
#[allow(clippy::too_many_arguments)]
pub fn encode_mp4_with_codec(
    frames: &Path,
    destination: &Path,
    tools: &ToolPaths,
    count: u32,
    width: u32,
    height: u32,
    codec: super::jobs::VideoCodec,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    encode_mp4_with_codec_at_rate(
        frames,
        destination,
        tools,
        count,
        width,
        height,
        codec,
        20,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn encode_mp4_with_codec_at_rate(
    frames: &Path,
    destination: &Path,
    tools: &ToolPaths,
    count: u32,
    width: u32,
    height: u32,
    codec: super::jobs::VideoCodec,
    frame_rate: u32,
    cancel: &AtomicBool,
) -> Result<ExportResult> {
    validate_frame_rate(frame_rate)?;
    validate_frames_dimensions(frames, count, width, height)?;
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err("MP4 dimensions must be even".into());
    }
    let scratch = Scratch::new(destination.parent().ok_or("destination has no parent")?)?;
    let encoded = scratch.0.join("output.mp4");
    let rate = frame_rate.to_string();
    let mut args = arguments(&[
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-framerate",
        &rate,
        "-start_number",
        "0",
        "-i",
        "frame_%04d.png",
        "-frames:v",
    ]);
    args.push(count.to_string());
    let encoder = video_encoder(tools, codec, cancel)?;
    args.extend(arguments(&["-c:v", &encoder]));
    match encoder.as_str() {
        "libaom-av1" => args.extend(arguments(&[
            "-crf",
            "30",
            "-b:v",
            "0",
            "-cpu-used",
            "6",
            "-row-mt",
            "1",
            "-threads",
            "8",
        ])),
        "libsvtav1" => args.extend(arguments(&["-crf", "30", "-preset", "8"])),
        "av1_nvenc" => args.extend(arguments(&["-cq", "30", "-b:v", "0", "-preset", "p5"])),
        _ => args.extend(arguments(&["-crf", "18"])),
    }
    args.extend(arguments(&[
        "-r",
        &rate,
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
    ]));
    args.push(encoded.to_string_lossy().into_owned());
    run_owned(
        &tools.ffmpeg,
        &args,
        Some(frames),
        cancel,
        Duration::from_secs(1200),
    )?;
    let mut probe = arguments(&[
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-count_frames",
        "-show_entries",
        "stream=width,height,nb_read_frames,duration,codec_name,avg_frame_rate",
        "-of",
        "json",
    ]);
    probe.push(encoded.to_string_lossy().into_owned());
    let bytes = run_owned(
        &tools.ffprobe,
        &probe,
        None,
        cancel,
        Duration::from_secs(60),
    )?;
    let info: serde_json::Value = serde_json::from_slice(&bytes)?;
    let stream = &info["streams"][0];
    let duration = stream["duration"]
        .as_str()
        .ok_or("ffprobe missing duration")?
        .parse::<f64>()?;
    let (rate_numerator, rate_denominator) = stream["avg_frame_rate"]
        .as_str()
        .and_then(|value| value.split_once('/'))
        .ok_or("ffprobe missing frame rate")?;
    let actual_rate = rate_numerator.parse::<f64>()? / rate_denominator.parse::<f64>()?;
    let expected_duration = f64::from(count) / f64::from(frame_rate);
    if stream["width"] != width
        || stream["height"] != height
        || stream["codec_name"]
            != match codec {
                super::jobs::VideoCodec::H264 => "h264",
                super::jobs::VideoCodec::Av1 => "av1",
            }
        || stream["nb_read_frames"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            != Some(count)
        || !duration.is_finite()
        || (duration - expected_duration).abs() > 0.001
        || !actual_rate.is_finite()
        || (actual_rate - f64::from(frame_rate)).abs() > 0.000_001
    {
        return Err("MP4 dimensions/frame count/duration/frame rate validation failed".into());
    }
    cancelled(cancel)?;
    publish(&encoded, destination)?;
    result(
        destination,
        width,
        height,
        count,
        (duration * 1000.0).round() as u64,
    )
}
