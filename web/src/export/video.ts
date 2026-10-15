// Video encoding with WebCodecs (hardware encoders where available) and
// mediabunny muxing. Works in workers.

import {
  BufferTarget,
  canEncodeVideo,
  Mp4OutputFormat,
  Output,
  QUALITY_VERY_HIGH,
  VideoSample,
  VideoSampleSource,
  WebMOutputFormat,
  type VideoCodec,
} from 'mediabunny';

export type VideoFormat = 'mp4-h264' | 'mp4-av1' | 'webm-vp9' | 'webm-av1';

export const VIDEO_FORMATS: Record<VideoFormat, { label: string; codec: VideoCodec; container: 'mp4' | 'webm'; mime: string; ext: string }> = {
  'mp4-h264': { label: 'MP4 (H.264, most compatible)', codec: 'avc', container: 'mp4', mime: 'video/mp4', ext: 'mp4' },
  'mp4-av1': { label: 'MP4 (AV1, smallest)', codec: 'av1', container: 'mp4', mime: 'video/mp4', ext: 'mp4' },
  'webm-vp9': { label: 'WebM (VP9)', codec: 'vp9', container: 'webm', mime: 'video/webm', ext: 'webm' },
  'webm-av1': { label: 'WebM (AV1)', codec: 'av1', container: 'webm', mime: 'video/webm', ext: 'webm' },
};

export async function supportedVideoFormats(width: number, height: number): Promise<VideoFormat[]> {
  const out: VideoFormat[] = [];
  for (const [key, f] of Object.entries(VIDEO_FORMATS) as [VideoFormat, (typeof VIDEO_FORMATS)[VideoFormat]][]) {
    try {
      if (await canEncodeVideo(f.codec, { width, height })) out.push(key);
    } catch {
      // unsupported
    }
  }
  return out;
}

export class VideoWriter {
  private constructor(
    private output: Output<Mp4OutputFormat | WebMOutputFormat, BufferTarget>,
    private source: VideoSampleSource,
    private width: number,
    private height: number,
    private fps: number,
  ) {}

  static async create(format: VideoFormat, width: number, height: number, fps: number): Promise<VideoWriter> {
    const f = VIDEO_FORMATS[format];
    const output = new Output({
      format: f.container === 'mp4' ? new Mp4OutputFormat({ fastStart: 'in-memory' }) : new WebMOutputFormat(),
      target: new BufferTarget(),
    });
    const source = new VideoSampleSource({ codec: f.codec, quality: QUALITY_VERY_HIGH, keyFrameInterval: 1 });
    output.addVideoTrack(source, { frameRate: fps });
    await output.start();
    return new VideoWriter(output, source, width, height, fps);
  }

  async addFrame(rgba: Uint8ClampedArray, index: number) {
    const sample = new VideoSample(rgba, {
      format: 'RGBA',
      codedWidth: this.width,
      codedHeight: this.height,
      timestamp: index / this.fps,
      duration: 1 / this.fps,
    });
    try {
      await this.source.add(sample);
    } finally {
      sample.close();
    }
  }

  async finish(): Promise<Uint8Array> {
    await this.output.finalize();
    const buf = this.output.target.buffer;
    if (!buf) throw new Error('Video encoder produced no data');
    return new Uint8Array(buf);
  }

  async cancel() {
    await this.output.cancel();
  }
}
