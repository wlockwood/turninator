use std::{fs, io::BufReader, path::Path, sync::atomic::AtomicBool};
use turninator::{
    export::{
        encode::{
            Scratch, encode_gif_dimensions_at_rate, encode_mp4_with_codec_at_rate,
            gif_frame_delays, inspect_gif_dimensions_at_rate,
        },
        jobs::VideoCodec,
    },
    render::ToolPaths,
};

#[test]
fn gif_delays_round_absolute_boundaries_without_accumulating_drift() {
    assert_eq!(gif_frame_delays(8, 24).unwrap(), [4, 4, 5, 4, 4, 4, 4, 4]);
    assert_eq!(gif_frame_delays(6, 30).unwrap(), [3, 4, 3, 3, 4, 3]);
    assert_eq!(gif_frame_delays(6, 60).unwrap(), [2, 1, 2, 2, 1, 2]);
    assert_eq!(gif_frame_delays(2, 20).unwrap(), [5, 5]);
    for fps in 1..=100 {
        let delays = gif_frame_delays(10_000, fps).unwrap();
        assert!(delays.iter().all(|&delay| delay >= 1));
        let actual: u64 = delays.iter().map(|&delay| u64::from(delay)).sum();
        assert!((actual as f64 - 1_000_000.0 / f64::from(fps)).abs() <= 0.5);
    }
    for (count, fps) in [(0, 20), (10_001, 20), (2, 0), (2, 101), (2, u32::MAX)] {
        assert!(gif_frame_delays(count, fps).is_err());
    }
}

fn write_frames(directory: &Path, count: u32) {
    for index in 0..count {
        let color = [
            (index * 31 % 256) as u8,
            (index * 67 % 256) as u8,
            (index * 97 % 256) as u8,
        ];
        image::RgbImage::from_pixel(24, 16, image::Rgb(color))
            .save(directory.join(format!("frame_{index:04}.png")))
            .unwrap();
    }
}

#[test]
fn actual_gif_encoder_preserves_requested_frames_and_quantized_duration() {
    let tools = ToolPaths::default();
    if !tools.ffmpeg.is_file() {
        eprintln!("FFmpeg unavailable; skipping actual encoding");
        return;
    }
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    write_frames(&scratch.0, 8);
    for (fps, expected) in [
        (20, vec![5; 8]),
        (24, vec![4, 4, 5, 4, 4, 4, 4, 4]),
        (30, vec![3, 4, 3, 3, 4, 3, 3, 4]),
        (60, vec![2, 1, 2, 2, 1, 2, 2, 1]),
        (100, vec![1; 8]),
        (1, vec![100; 8]),
    ] {
        let destination = scratch.0.join(format!("{fps}.gif"));
        let result = encode_gif_dimensions_at_rate(
            &scratch.0,
            &destination,
            &tools,
            8,
            24,
            16,
            fps,
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options
            .read_info(BufReader::new(fs::File::open(&destination).unwrap()))
            .unwrap();
        let mut delays = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            let index = delays.len() as u32;
            assert_eq!(
                &frame.buffer[..4],
                &[
                    (index * 31 % 256) as u8,
                    (index * 67 % 256) as u8,
                    (index * 97 % 256) as u8,
                    255
                ],
                "decoded color at frame {index}, FPS {fps}"
            );
            delays.push(frame.delay);
        }
        assert_eq!(delays, expected, "FPS {fps}");
        assert_eq!(decoder.repeat(), gif::Repeat::Infinite);
        assert_eq!(
            result.duration_ms,
            expected.iter().map(|&x| u64::from(x) * 10).sum::<u64>()
        );
        inspect_gif_dimensions_at_rate(&destination, 24, 16, 8, fps).unwrap();
        assert!(inspect_gif_dimensions_at_rate(&destination, 24, 16, 7, fps).is_err());
    }
}

#[test]
fn actual_mp4_encoder_reports_requested_rate_and_rounded_duration() {
    let tools = ToolPaths::default();
    if !tools.ffmpeg.is_file() || !tools.ffprobe.is_file() {
        eprintln!("FFmpeg unavailable; skipping actual encoding");
        return;
    }
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    write_frames(&scratch.0, 8);
    for (fps, duration_ms) in [(24, 333), (30, 267), (60, 133), (100, 80), (1, 8000)] {
        let destination = scratch.0.join(format!("{fps}.mp4"));
        let result = encode_mp4_with_codec_at_rate(
            &scratch.0,
            &destination,
            &tools,
            8,
            24,
            16,
            VideoCodec::H264,
            fps,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.duration_ms, duration_ms);
        let output = std::process::Command::new(&tools.ffprobe)
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_entries",
                "stream=avg_frame_rate,nb_read_frames,duration",
                "-of",
                "json",
            ])
            .arg(&destination)
            .output()
            .unwrap();
        assert!(output.status.success());
        let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(info["streams"][0]["avg_frame_rate"], format!("{fps}/1"));
        assert_eq!(info["streams"][0]["nb_read_frames"], "8");
        let decoded = std::process::Command::new(&tools.ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&destination)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        assert!(decoded.status.success());
        assert_eq!(decoded.stdout.len(), 8 * 24 * 16 * 3);
        for (index, frame) in decoded.stdout.chunks_exact(24 * 16 * 3).enumerate() {
            let color = [
                (index * 31 % 256) as u8,
                (index * 67 % 256) as u8,
                (index * 97 % 256) as u8,
            ];
            assert!(
                frame[..3]
                    .iter()
                    .zip(color)
                    .all(|(&actual, expected)| actual.abs_diff(expected) <= 8),
                "decoded MP4 color at frame {index}, FPS {fps}"
            );
        }
    }
}

#[test]
fn invalid_rates_reject_before_reading_frames_or_replacing_destination() {
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    let tools = ToolPaths {
        blender: "unused".into(),
        ffmpeg: "unused".into(),
        ffprobe: "unused".into(),
    };
    let destination = scratch.0.join("keep.output");
    fs::write(&destination, b"preserve").unwrap();
    for fps in [0, 101, u32::MAX] {
        let gif = encode_gif_dimensions_at_rate(
            &scratch.0,
            &destination,
            &tools,
            2,
            24,
            16,
            fps,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(gif.to_string().contains("frame rate"));
        let mp4 = encode_mp4_with_codec_at_rate(
            &scratch.0,
            &destination,
            &tools,
            2,
            24,
            16,
            VideoCodec::H264,
            fps,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(mp4.to_string().contains("frame rate"));
        assert_eq!(fs::read(&destination).unwrap(), b"preserve");
    }
}
