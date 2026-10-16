//! Disk-backed animation playback and a coalescing loader for render progress images.
use crate::Result;
use eframe::egui;
use image::AnimationDecoder;
use std::{
    fs::File,
    io::{BufReader, Seek},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

// At most three display-sized frames wait in RAM, independently of clip length.
const QUEUED_FRAMES: usize = 3;
type FrameResult = std::result::Result<PlaybackFrame, String>;

pub(super) struct PlaybackFrame {
    image: egui::ColorImage,
    duration: Duration,
}
impl PlaybackFrame {
    fn from_buffer(buffer: image::RgbaImage, duration: Duration) -> Self {
        let buffer = display_buffer(buffer);
        Self {
            image: egui::ColorImage::from_rgba_unmultiplied(
                [buffer.width() as usize, buffer.height() as usize],
                buffer.as_raw(),
            ),
            duration,
        }
    }
}

struct FrameStream {
    frames: Receiver<FrameResult>,
    // Dropping the receiver releases a producer blocked on a full queue. The
    // detached worker then exits after its current frame, without blocking UI.
    _worker: JoinHandle<()>,
}
impl FrameStream {
    fn open(file: File, ctx: egui::Context) -> Self {
        let (sender, frames) = mpsc::sync_channel(QUEUED_FRAMES);
        let worker = thread::spawn(move || {
            if let Err(error) = stream_gif(file, &sender, &ctx) {
                let _ = sender.send(Err(error.to_string()));
                ctx.request_repaint();
            }
        });
        Self {
            frames,
            _worker: worker,
        }
    }
}

fn stream_gif(mut file: File, sender: &SyncSender<FrameResult>, ctx: &egui::Context) -> Result<()> {
    loop {
        // Keep the same open file across loops even if a later export replaces
        // the destination. A fresh decoder also resets GIF disposal state.
        file.rewind()?;
        let decoder = image::codecs::gif::GifDecoder::new(BufReader::new(&mut file))?;
        let mut count = 0;
        for frame in decoder.into_frames() {
            let frame = frame?;
            let (n, d) = frame.delay().numer_denom_ms();
            let frame = PlaybackFrame::from_buffer(
                frame.into_buffer(),
                Duration::from_secs_f64((n as f64 / d.max(1) as f64 / 1000.).max(0.01)),
            );
            if sender.send(Ok(frame)).is_err() {
                return Ok(());
            }
            ctx.request_repaint();
            count += 1;
        }
        if count == 0 {
            return Err("Animation contains no frames".into());
        }
        if count == 1 {
            return Ok(());
        }
    }
}

pub(super) struct DecodedOutput {
    first: PlaybackFrame,
    stream: Option<FrameStream>,
}

pub(super) fn decode_output(path: &Path, ctx: egui::Context) -> Result<DecodedOutput> {
    if path
        .extension()
        .is_some_and(|v| v.eq_ignore_ascii_case("gif"))
    {
        let stream = FrameStream::open(File::open(path)?, ctx);
        let first = stream.frames.recv().map_err(|_| "GIF decoder stopped")??;
        Ok(DecodedOutput {
            first,
            stream: Some(stream),
        })
    } else {
        Ok(DecodedOutput {
            first: decode_image(path)?,
            stream: None,
        })
    }
}

fn decode_image(path: &Path) -> Result<PlaybackFrame> {
    Ok(PlaybackFrame::from_buffer(
        image::open(path)?.to_rgba8(),
        Duration::from_secs(1),
    ))
}

pub(super) struct Playback {
    pub texture: egui::TextureHandle,
    pub playing: bool,
    stream: Option<FrameStream>,
    duration: Duration,
    next: Instant,
    buffering: bool,
}
impl Playback {
    pub fn new(ctx: &egui::Context, decoded: DecodedOutput) -> Self {
        Self {
            texture: ctx.load_texture(
                "encoded-output",
                decoded.first.image,
                egui::TextureOptions::LINEAR,
            ),
            playing: true,
            stream: decoded.stream,
            duration: decoded.first.duration,
            next: Instant::now() + decoded.first.duration,
            buffering: false,
        }
    }
    pub fn animated(&self) -> bool {
        self.stream.is_some()
    }
    pub fn resume(&mut self) {
        self.next = Instant::now() + self.duration;
        self.buffering = false;
    }
    pub fn advance(&mut self, ctx: &egui::Context) -> std::result::Result<(), String> {
        self.advance_at(ctx, Instant::now())
    }
    fn advance_at(&mut self, ctx: &egui::Context, now: Instant) -> std::result::Result<(), String> {
        if !self.playing || self.stream.is_none() {
            return Ok(());
        }
        // Limit catch-up work even when the UI was obstructed for a long time.
        for _ in 0..QUEUED_FRAMES {
            if now < self.next {
                break;
            }
            match self.stream.as_ref().unwrap().frames.try_recv() {
                Ok(Ok(frame)) => {
                    self.texture.set(frame.image, egui::TextureOptions::LINEAR);
                    self.duration = frame.duration;
                    if self.buffering {
                        self.next = now + self.duration;
                        self.buffering = false;
                    } else {
                        self.next += self.duration;
                    }
                }
                Ok(Err(error)) => {
                    self.stream = None;
                    self.playing = false;
                    return Err(format!("Animation playback failed: {error}"));
                }
                Err(TryRecvError::Disconnected) => {
                    self.stream = None;
                    return Ok(());
                }
                Err(TryRecvError::Empty) => {
                    // Hold the current image during an underrun. Rebase the
                    // clock so the next frame still receives its saved delay.
                    self.buffering = true;
                    ctx.request_repaint_after(Duration::from_millis(10));
                    return Ok(());
                }
            }
        }
        if now >= self.next {
            self.next = now + self.duration;
        }
        ctx.request_repaint_after(
            self.next
                .saturating_duration_since(now)
                .max(Duration::from_millis(1)),
        );
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RenderTag {
    id: u64,
    revision: u64,
}
struct RenderImage {
    tag: RenderTag,
    path: PathBuf,
}
type RenderImageResult = (RenderTag, FrameResult);

pub(super) struct LivePreview {
    job: Option<RenderTag>,
    image: Option<Playback>,
    pending: Arc<Mutex<Option<RenderImage>>>,
    ready: Arc<Mutex<Option<RenderImageResult>>>,
    wake: SyncSender<()>,
}
impl LivePreview {
    pub fn new(ctx: egui::Context) -> Self {
        let pending = Arc::new(Mutex::new(None::<RenderImage>));
        let ready = Arc::new(Mutex::new(None));
        let (wake, requests) = mpsc::sync_channel(1);
        let worker_pending = pending.clone();
        let worker_ready = ready.clone();
        thread::spawn(move || {
            while requests.recv().is_ok() {
                let Some(request) = worker_pending.lock().unwrap().take() else {
                    continue;
                };
                let frame = decode_image(&request.path).map_err(|error| error.to_string());
                *worker_ready.lock().unwrap() = Some((request.tag, frame));
                ctx.request_repaint();
            }
        });
        Self {
            job: None,
            image: None,
            pending,
            ready,
            wake,
        }
    }
    pub fn start(&mut self, id: u64, revision: u64) {
        self.stop();
        self.job = Some(RenderTag { id, revision });
    }
    pub fn stop(&mut self) {
        self.job = None;
        self.image = None;
        self.pending.lock().unwrap().take();
        self.ready.lock().unwrap().take();
    }
    pub fn submit(&self, id: u64, revision: u64, path: PathBuf) {
        let tag = RenderTag { id, revision };
        if self.job != Some(tag) {
            return;
        }
        // Reuse bursts and fast renders replace this single pending path. The
        // renderer never waits for image decoding or queues pixel buffers.
        *self.pending.lock().unwrap() = Some(RenderImage { tag, path });
        let _ = self.wake.try_send(());
    }
    pub fn receive(
        &mut self,
        ctx: &egui::Context,
        revision: u64,
    ) -> std::result::Result<bool, String> {
        let Some((tag, result)) = self.ready.lock().unwrap().take() else {
            return Ok(false);
        };
        if self.job != Some(tag) || tag.revision != revision {
            return Ok(false);
        }
        match result {
            Ok(first) => {
                self.image = Some(Playback::new(
                    ctx,
                    DecodedOutput {
                        first,
                        stream: None,
                    },
                ));
                Ok(true)
            }
            Err(error) => Err(format!("Live render preview unavailable: {error}")),
        }
    }
    pub fn current(&self, revision: u64) -> Option<&Playback> {
        self.job
            .filter(|tag| tag.revision == revision)
            .and(self.image.as_ref())
    }
    pub fn current_mut(&mut self, revision: u64) -> Option<&mut Playback> {
        self.job
            .filter(|tag| tag.revision == revision)
            .and(self.image.as_mut())
    }
}

fn display_buffer(buffer: image::RgbaImage) -> image::RgbaImage {
    let largest = buffer.width().max(buffer.height());
    if largest <= 1280 {
        buffer
    } else {
        let scale = 1280. / largest as f64;
        image::imageops::resize(
            &buffer,
            (buffer.width() as f64 * scale).round().max(1.) as u32,
            (buffer.height() as f64 * scale).round().max(1.) as u32,
            image::imageops::FilterType::Triangle,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::encode::Scratch;

    fn animated_fixture(directory: &Path) -> PathBuf {
        let path = directory.join("disposal.gif");
        let mut encoder = gif::Encoder::new(
            File::create(&path).unwrap(),
            3,
            1,
            &[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
        )
        .unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for (left, pixels, delay, dispose) in [
            (0, vec![1, 1, 1], 2, gif::DisposalMethod::Keep),
            (1, vec![2], 3, gif::DisposalMethod::Previous),
            (2, vec![3], 4, gif::DisposalMethod::Background),
            (0, vec![2], 5, gif::DisposalMethod::Keep),
        ] {
            encoder
                .write_frame(&gif::Frame {
                    left,
                    width: pixels.len() as u16,
                    height: 1,
                    buffer: pixels.into(),
                    delay,
                    dispose,
                    transparent: Some(0),
                    ..Default::default()
                })
                .unwrap();
        }
        path
    }

    #[test]
    fn streaming_preserves_frame_order_delays_and_disposal_across_loops() {
        use egui::Color32;
        let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
        let decoded =
            decode_output(&animated_fixture(&scratch.0), egui::Context::default()).unwrap();
        let red = Color32::RED;
        let green = Color32::GREEN;
        let blue = Color32::BLUE;
        let expected = [
            ([red, red, red], 20),
            ([red, green, red], 30),
            ([red, red, blue], 40),
            ([green, red, Color32::TRANSPARENT], 50),
        ];
        let stream = decoded.stream.unwrap();
        let mut first = Some(decoded.first);
        for index in 0..12 {
            let frame = first.take().unwrap_or_else(|| {
                stream
                    .frames
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap()
            });
            let (pixels, delay) = expected[index % 4];
            assert_eq!(frame.image.size, [3, 1]);
            assert_eq!(frame.image.pixels, pixels, "frame {index}");
            assert_eq!(frame.duration, Duration::from_millis(delay));
        }
    }

    #[test]
    fn dropping_a_paused_stream_releases_the_decoder_and_file() {
        let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
        let path = animated_fixture(&scratch.0);
        let decoded = decode_output(&path, egui::Context::default()).unwrap();
        let FrameStream { frames, _worker } = decoded.stream.unwrap();
        // No further consumption: the producer may be blocked on its full queue.
        drop(frames);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !_worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            _worker.is_finished(),
            "decoder must stop after receiver drop"
        );
        _worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    fn clock_playback(ctx: &egui::Context) -> (Playback, SyncSender<FrameResult>) {
        let (sender, frames) = mpsc::sync_channel(3);
        let playback = Playback::new(
            ctx,
            DecodedOutput {
                first: PlaybackFrame::from_buffer(
                    image::RgbaImage::new(2, 1),
                    Duration::from_millis(20),
                ),
                stream: Some(FrameStream {
                    frames,
                    _worker: thread::spawn(|| {}),
                }),
            },
        );
        (playback, sender)
    }
    fn next_frame() -> PlaybackFrame {
        PlaybackFrame::from_buffer(image::RgbaImage::new(3, 1), Duration::from_millis(30))
    }

    #[test]
    fn pause_holds_the_current_image_and_resume_uses_its_delay() {
        let ctx = egui::Context::default();
        let (mut playback, sender) = clock_playback(&ctx);
        sender.send(Ok(next_frame())).unwrap();
        playback.playing = false;
        playback
            .advance_at(&ctx, playback.next + Duration::from_secs(100))
            .unwrap();
        assert_eq!(playback.texture.size(), [2, 1]);
        playback.playing = true;
        let before = Instant::now();
        playback.resume();
        assert!(playback.next >= before + Duration::from_millis(20));
        playback
            .advance_at(&ctx, playback.next - Duration::from_millis(1))
            .unwrap();
        assert_eq!(playback.texture.size(), [2, 1]);
        let due = playback.next;
        playback.advance_at(&ctx, due).unwrap();
        assert_eq!(playback.texture.size(), [3, 1]);
        assert_eq!(playback.next, due + Duration::from_millis(30));
    }

    #[test]
    fn disk_underrun_holds_the_image_and_rebases_the_next_frame_delay() {
        let ctx = egui::Context::default();
        let (mut playback, sender) = clock_playback(&ctx);
        let due = playback.next;
        playback.advance_at(&ctx, due).unwrap();
        assert_eq!(playback.texture.size(), [2, 1]);
        sender.send(Ok(next_frame())).unwrap();
        let ready_at = due + Duration::from_secs(10);
        playback.advance_at(&ctx, ready_at).unwrap();
        assert_eq!(playback.texture.size(), [3, 1]);
        assert_eq!(playback.next, ready_at + Duration::from_millis(30));
    }

    #[test]
    fn corrupt_later_frames_report_an_error_and_hold_the_last_image() {
        let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
        let path = scratch.0.join("corrupt.gif");
        {
            let mut encoder =
                gif::Encoder::new(File::create(&path).unwrap(), 1, 1, &[255, 0, 0, 0, 0, 0])
                    .unwrap();
            encoder
                .write_frame(&gif::Frame {
                    width: 1,
                    height: 1,
                    delay: 2,
                    buffer: vec![0].into(),
                    ..Default::default()
                })
                .unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.pop(), Some(0x3b)); // Replace trailer with a truncated frame.
        bytes.extend([0x2c, 0x00]);
        std::fs::write(&path, bytes).unwrap();
        let ctx = egui::Context::default();
        let decoded = decode_output(&path, ctx.clone()).unwrap();
        let mut playback = Playback::new(&ctx, decoded);
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = loop {
            if let Err(error) = playback.advance_at(&ctx, playback.next) {
                break error;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        };
        assert!(error.contains("Animation playback failed"));
        assert!(!playback.playing);
        assert!(!playback.animated());
        assert_eq!(playback.texture.size(), [1, 1]);
    }

    fn write_image(path: &Path, width: u32) {
        image::RgbaImage::from_pixel(width, 2, image::Rgba([255, 0, 0, 255]))
            .save(path)
            .unwrap();
    }
    fn wait_for_live(loader: &mut LivePreview, ctx: &egui::Context, revision: u64, width: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            loader.receive(ctx, revision).unwrap();
            if loader
                .current(revision)
                .is_some_and(|p| p.texture.size() == [width, 2])
            {
                return;
            }
            assert!(Instant::now() < deadline, "live image did not arrive");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn live_images_update_before_the_render_job_finishes() {
        let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
        let first = scratch.0.join("first.png");
        let second = scratch.0.join("second.png");
        write_image(&first, 4);
        write_image(&second, 8);
        let ctx = egui::Context::default();
        let mut loader = LivePreview::new(ctx.clone());
        loader.start(1, 10);
        loader.submit(1, 10, first);
        wait_for_live(&mut loader, &ctx, 10, 4);
        loader.submit(1, 10, second);
        wait_for_live(&mut loader, &ctx, 10, 8);
        assert!(!loader.current(10).unwrap().animated());
    }

    #[test]
    fn cancelled_or_superseded_live_images_cannot_replace_the_current_display() {
        let ctx = egui::Context::default();
        let mut loader = LivePreview::new(ctx.clone());
        loader.start(1, 10);
        loader.stop();
        *loader.ready.lock().unwrap() = Some((
            RenderTag {
                id: 1,
                revision: 10,
            },
            Ok(next_frame()),
        ));
        assert_eq!(loader.receive(&ctx, 10), Ok(false));
        assert!(loader.current(10).is_none());
        loader.start(2, 10);
        *loader.ready.lock().unwrap() = Some((
            RenderTag {
                id: 1,
                revision: 10,
            },
            Ok(next_frame()),
        ));
        assert_eq!(loader.receive(&ctx, 10), Ok(false));
        assert!(loader.current(10).is_none());
        *loader.ready.lock().unwrap() = Some((
            RenderTag {
                id: 2,
                revision: 10,
            },
            Ok(next_frame()),
        ));
        assert_eq!(loader.receive(&ctx, 11), Ok(false));
        assert!(loader.current(11).is_none());
        loader.submit(1, 10, PathBuf::from("obsolete.png"));
        assert!(loader.pending.lock().unwrap().is_none());
    }

    #[test]
    fn live_render_bursts_eventually_display_the_newest_image() {
        let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
        let ctx = egui::Context::default();
        let mut loader = LivePreview::new(ctx.clone());
        loader.start(1, 10);
        for index in 1..=20 {
            let path = scratch.0.join(format!("{index}.png"));
            write_image(&path, index);
            loader.submit(1, 10, path);
        }
        wait_for_live(&mut loader, &ctx, 10, 20);
    }

    #[test]
    #[ignore = "requires TURNINATOR_PLAYBACK_GIF pointing to the local 160-frame DRA Case export"]
    fn local_dra_case_streams_two_complete_loops() {
        let path = PathBuf::from(std::env::var_os("TURNINATOR_PLAYBACK_GIF").unwrap());
        let decoded = decode_output(&path, egui::Context::default()).unwrap();
        assert_eq!(decoded.first.image.size, [960, 960]);
        let first_pixels = decoded.first.image.pixels.clone();
        let stream = decoded.stream.unwrap();
        for index in 1..=320 {
            let frame = stream
                .frames
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            assert_eq!(frame.image.size, [960, 960]);
            assert_eq!(frame.duration, Duration::from_millis(50));
            if index == 160 || index == 320 {
                assert_eq!(frame.image.pixels, first_pixels, "loop restart {index}");
            }
        }
        eprintln!("Decoded 321 DRA Case frames across two loop restarts with a bounded queue");
    }
}
