use std::f32::consts::PI;

use crate::audio::source::AudioSource;

pub const EQ_BAND_FREQS: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

#[derive(Clone, Debug)]
pub struct EqConfig {
    pub enabled: bool,
    pub gains_db: [f32; 10],
}

impl Default for EqConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gains_db: [0.0; 10],
        }
    }
}

impl EqConfig {
    pub fn is_active(&self) -> bool {
        self.enabled && self.gains_db.iter().any(|gain| gain.abs() > 0.01)
    }
}

pub struct EqualizerSource<S> {
    source: S,
    eq: Option<Equalizer>,
}

impl<S: AudioSource> EqualizerSource<S> {
    pub fn new(source: S, config: EqConfig) -> Self {
        if config.is_active() {
            let eq = Equalizer::new(
                source.channel_count(),
                source.sample_rate(),
                config.gains_db,
            );
            Self {
                source,
                eq: Some(eq),
            }
        } else {
            Self { source, eq: None }
        }
    }
}

impl<S: AudioSource> AudioSource for EqualizerSource<S> {
    fn write(&mut self, output: &mut [f32]) -> usize {
        let written = self.source.write(output);
        if let Some(eq) = &mut self.eq {
            eq.process(&mut output[..written]);
        }
        written
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }
}

struct Equalizer {
    filters: Vec<Vec<Biquad>>,
    channels: usize,
}

impl Equalizer {
    fn new(channels: usize, sample_rate: u32, gains_db: [f32; 10]) -> Self {
        let mut filters = Vec::with_capacity(channels);
        for _ in 0..channels {
            let mut band_filters = Vec::with_capacity(EQ_BAND_FREQS.len());
            for (freq, gain) in EQ_BAND_FREQS.iter().zip(gains_db.iter()) {
                band_filters.push(Biquad::peaking(sample_rate, *freq, 1.0, *gain));
            }
            filters.push(band_filters);
        }
        Self { filters, channels }
    }

    fn process(&mut self, samples: &mut [f32]) {
        if self.channels == 0 {
            return;
        }
        for (index, sample) in samples.iter_mut().enumerate() {
            let channel = index % self.channels;
            let mut value = *sample;
            for filter in &mut self.filters[channel] {
                value = filter.process(value);
            }
            *sample = value;
        }
    }
}

struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn peaking(sample_rate: u32, freq: f32, q: f32, gain_db: f32) -> Self {
        let sample_rate = sample_rate as f32;
        let nyquist = sample_rate * 0.5;
        let freq = freq.clamp(10.0, nyquist * 0.98);
        let q = q.max(0.1);

        let a = 10.0_f32.powf(gain_db / 40.0);
        let omega = 2.0 * PI * freq / sample_rate;
        let sin = omega.sin();
        let cos = omega.cos();
        let alpha = sin / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        output
    }
}
