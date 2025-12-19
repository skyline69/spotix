use crate::audio::resample::ResamplingSpec;

use crossbeam_channel::{Receiver, Sender, bounded};

use super::resample::{AudioResampler, ResamplingQuality};

/// Types that can produce audio samples in `f32` format. `Send`able across
/// threads.
pub trait AudioSource: Send + 'static {
    /// Write at most of `output.len()` samples into the `output`. Returns the
    /// number of written samples. Should take care to always output a full
    /// frame, and should _never_ block.
    fn write(&mut self, output: &mut [f32]) -> usize;
    fn channel_count(&self) -> usize;
    fn sample_rate(&self) -> u32;
}

impl AudioSource for Box<dyn AudioSource> {
    fn write(&mut self, output: &mut [f32]) -> usize {
        self.as_mut().write(output)
    }

    fn channel_count(&self) -> usize {
        self.as_ref().channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.as_ref().sample_rate()
    }
}

/// Empty audio source. Does not produce any samples.
pub struct Empty;

impl AudioSource for Empty {
    fn write(&mut self, _output: &mut [f32]) -> usize {
        0
    }

    fn channel_count(&self) -> usize {
        0
    }

    fn sample_rate(&self) -> u32 {
        0
    }
}

pub struct StereoMappedSource<S> {
    source: S,
    input_channels: usize,
    output_channels: usize,
    buffer: Vec<f32>,
}

impl<S> StereoMappedSource<S>
where
    S: AudioSource,
{
    pub fn new(source: S, output_channels: usize) -> Self {
        const BUFFER_SIZE: usize = 16 * 1024;

        let input_channels = source.channel_count();
        Self {
            source,
            input_channels,
            output_channels,
            buffer: vec![0.0; BUFFER_SIZE],
        }
    }
}

impl<S> AudioSource for StereoMappedSource<S>
where
    S: AudioSource,
{
    fn write(&mut self, output: &mut [f32]) -> usize {
        let input_max = (output.len() / self.output_channels) * self.input_channels;
        let buffer_max = input_max.min(self.buffer.len());
        let written = self.source.write(&mut self.buffer[..buffer_max]);
        let input = &self.buffer[..written];
        let input_frames = input.chunks_exact(self.input_channels);
        let output_frames = output.chunks_exact_mut(self.output_channels);
        for (i, o) in input_frames.zip(output_frames) {
            o[0] = i[0];
            o[1] = i[1];
            // Assume the rest is is implicitly silence.
        }
        output.len()
    }

    fn channel_count(&self) -> usize {
        self.output_channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
}

pub struct MonoSource<S> {
    source: S,
    input_channels: usize,
    buffer: Vec<f32>,
}

impl<S> MonoSource<S>
where
    S: AudioSource,
{
    pub fn new(source: S) -> Self {
        const BUFFER_SIZE: usize = 16 * 1024;

        let input_channels = source.channel_count();
        Self {
            source,
            input_channels,
            buffer: vec![0.0; BUFFER_SIZE],
        }
    }
}

impl<S> AudioSource for MonoSource<S>
where
    S: AudioSource,
{
    fn write(&mut self, output: &mut [f32]) -> usize {
        if self.input_channels == 0 {
            return 0;
        }
        if self.input_channels == 1 {
            return self.source.write(output);
        }
        let frames = output.len();
        let input_needed = frames * self.input_channels;
        if self.buffer.len() < input_needed {
            self.buffer.resize(input_needed, 0.0);
        }
        let written = self.source.write(&mut self.buffer[..input_needed]);
        let written_frames = written / self.input_channels;
        for (frame, out_sample) in output.iter_mut().enumerate().take(written_frames) {
            let base = frame * self.input_channels;
            let mut sum = 0.0;
            for ch in 0..self.input_channels {
                sum += self.buffer[base + ch];
            }
            *out_sample = sum / self.input_channels as f32;
        }
        output[written_frames..].iter_mut().for_each(|s| *s = 0.0);
        written_frames
    }

    fn channel_count(&self) -> usize {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
}

pub struct MonoMappedSource<S> {
    source: S,
    output_channels: usize,
    buffer: Vec<f32>,
}

impl<S> MonoMappedSource<S>
where
    S: AudioSource,
{
    pub fn new(source: S, output_channels: usize) -> Self {
        const BUFFER_SIZE: usize = 16 * 1024;

        Self {
            source,
            output_channels,
            buffer: vec![0.0; BUFFER_SIZE],
        }
    }
}

impl<S> AudioSource for MonoMappedSource<S>
where
    S: AudioSource,
{
    fn write(&mut self, output: &mut [f32]) -> usize {
        let frames = output.len() / self.output_channels;
        if frames == 0 {
            return 0;
        }
        if self.buffer.len() < frames {
            self.buffer.resize(frames, 0.0);
        }
        let written_frames = self.source.write(&mut self.buffer[..frames]);
        let output_frames = output.chunks_exact_mut(self.output_channels);
        let mut written_samples = 0;
        for (value, out) in self.buffer[..written_frames].iter().zip(output_frames) {
            for sample in out.iter_mut() {
                *sample = *value;
            }
            written_samples += self.output_channels;
        }
        output[written_samples..].iter_mut().for_each(|s| *s = 0.0);
        written_samples
    }

    fn channel_count(&self) -> usize {
        self.output_channels
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
}

pub struct ResampledSource<S> {
    source: S,
    resampler: AudioResampler,
    inp: Buf,
    out: Buf,
}

pub enum CrossfadeCommand {
    ReplaceSource(Box<dyn AudioSource>),
    StartCrossfade {
        next: Box<dyn AudioSource>,
        duration_frames: u64,
    },
    Clear,
}

pub struct CrossfadeSource {
    receiver: Receiver<CrossfadeCommand>,
    current: Box<dyn AudioSource>,
    next: Option<Box<dyn AudioSource>>,
    fade: Option<FadeState>,
    buffer_a: Vec<f32>,
    buffer_b: Vec<f32>,
    channels: usize,
    sample_rate: u32,
}

struct FadeState {
    total_frames: u64,
    pos_frames: u64,
}

impl CrossfadeSource {
    pub fn new(initial: Box<dyn AudioSource>) -> (Self, Sender<CrossfadeCommand>) {
        let (sender, receiver) = bounded(8);
        let channels = initial.channel_count();
        let sample_rate = initial.sample_rate();
        let source = Self {
            receiver,
            current: initial,
            next: None,
            fade: None,
            buffer_a: Vec::new(),
            buffer_b: Vec::new(),
            channels,
            sample_rate,
        };
        (source, sender)
    }

    fn drain_commands(&mut self) {
        while let Ok(msg) = self.receiver.try_recv() {
            match msg {
                CrossfadeCommand::ReplaceSource(source) => {
                    self.channels = source.channel_count();
                    self.sample_rate = source.sample_rate();
                    self.current = source;
                    self.next = None;
                    self.fade = None;
                }
                CrossfadeCommand::StartCrossfade {
                    next,
                    duration_frames,
                } => {
                    if duration_frames == 0 {
                        self.channels = next.channel_count();
                        self.sample_rate = next.sample_rate();
                        self.current = next;
                        self.next = None;
                        self.fade = None;
                        continue;
                    }
                    self.next = Some(next);
                    self.fade = Some(FadeState {
                        total_frames: duration_frames,
                        pos_frames: 0,
                    });
                }
                CrossfadeCommand::Clear => {
                    self.current = Box::new(Empty);
                    self.next = None;
                    self.fade = None;
                }
            }
        }
    }

    fn ensure_buffer_sizes(&mut self, len: usize) {
        if self.buffer_a.len() < len {
            self.buffer_a.resize(len, 0.0);
        }
        if self.buffer_b.len() < len {
            self.buffer_b.resize(len, 0.0);
        }
    }
}

impl AudioSource for CrossfadeSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        self.drain_commands();

        if self.fade.is_some() {
            if self.channels == 0 {
                return 0;
            }
            let frames = output.len() / self.channels;
            if frames == 0 {
                return 0;
            }
            let max_len = frames * self.channels;
            self.ensure_buffer_sizes(max_len);

            let mut fade = self.fade.take().expect("fade state present");
            let current_written = self.current.write(&mut self.buffer_a[..max_len]);
            self.buffer_a[current_written..max_len].fill(0.0);

            let next_written = self
                .next
                .as_mut()
                .map(|src| src.write(&mut self.buffer_b[..max_len]))
                .unwrap_or(0);
            self.buffer_b[next_written..max_len].fill(0.0);

            let total_frames = fade.total_frames.max(1) as f32;
            for frame in 0..frames {
                let t = ((fade.pos_frames + frame as u64) as f32 / total_frames).min(1.0);
                let from_gain = 1.0 - t;
                let to_gain = t;
                let base = frame * self.channels;
                for ch in 0..self.channels {
                    let idx = base + ch;
                    output[idx] = self.buffer_a[idx] * from_gain + self.buffer_b[idx] * to_gain;
                }
            }
            output[max_len..].iter_mut().for_each(|s| *s = 0.0);

            fade.pos_frames += frames as u64;
            if fade.pos_frames >= fade.total_frames {
                if let Some(next) = self.next.take() {
                    self.channels = next.channel_count();
                    self.sample_rate = next.sample_rate();
                    self.current = next;
                }
                self.fade = None;
            } else {
                self.fade = Some(fade);
            }

            max_len
        } else {
            self.current.write(output)
        }
    }

    fn channel_count(&self) -> usize {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

impl<S> ResampledSource<S> {
    pub fn new(source: S, output_sample_rate: u32, quality: ResamplingQuality) -> Self
    where
        S: AudioSource,
    {
        const BUFFER_SIZE: usize = 1024;

        let spec = ResamplingSpec {
            channels: source.channel_count(),
            input_rate: source.sample_rate(),
            output_rate: output_sample_rate,
        };
        let inp_buf = vec![0.0; BUFFER_SIZE];
        let out_buf = vec![0.0; spec.output_size(BUFFER_SIZE)];
        Self {
            resampler: AudioResampler::new(quality, spec).unwrap(),
            source,
            inp: Buf {
                buf: inp_buf,
                start: 0,
                end: 0,
            },
            out: Buf {
                buf: out_buf,
                start: 0,
                end: 0,
            },
        }
    }
}

impl<S> AudioSource for ResampledSource<S>
where
    S: AudioSource,
{
    fn write(&mut self, output: &mut [f32]) -> usize {
        let mut total = 0;

        while total < output.len() {
            if self.out.is_empty() {
                if self.inp.is_empty() {
                    let n = self.source.write(&mut self.inp.buf);
                    self.inp.buf[n..].iter_mut().for_each(|s| *s = 0.0);
                    self.inp.start = 0;
                    self.inp.end = self.inp.buf.len();
                }
                let (inp_consumed, out_written) = self
                    .resampler
                    .process(&self.inp.buf[self.inp.start..], &mut self.out.buf)
                    .unwrap();
                self.inp.start += inp_consumed;
                self.out.start = 0;
                self.out.end = out_written;
            }
            let source = self.out.get();
            let target = &mut output[total..];
            let to_write = self.out.len().min(target.len());
            target[..to_write].copy_from_slice(&source[..to_write]);
            total += to_write;
            self.out.start += to_write;
        }

        total
    }

    fn channel_count(&self) -> usize {
        self.resampler.spec.channels
    }

    fn sample_rate(&self) -> u32 {
        self.resampler.spec.output_rate
    }
}

struct Buf {
    buf: Vec<f32>,
    start: usize,
    end: usize,
}

impl Buf {
    fn get(&self) -> &[f32] {
        &self.buf[self.start..self.end]
    }

    fn len(&self) -> usize {
        self.end - self.start
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}
