//! Fixed native sing-along preset: four stems to stereo, then a lookahead limiter.
//! No platform binary or runtime DSP dependency is used.

pub(super) const BLOCK_FRAMES: usize = 512;
pub(super) const LATENCY_FRAMES: usize = 219;
const RING: usize = 440;
const LIMIT: f32 = f32::from_bits(0x3f7d11d1);
const RELEASE_SAMPLES: f32 = (50.0_f32 / 1000.0) * 44_100.0;

pub(super) struct Mixer {
    frame: usize,
    buffer: [f32; RING],
    peaks: [usize; RING],
    release: [f32; RING],
    position: usize,
    attenuation: f32,
    delta: f32,
    head: usize,
    count: usize,
}
impl Mixer {
    pub(super) fn new() -> Self {
        Self {
            frame: 0,
            buffer: [0.; RING],
            peaks: [usize::MAX; RING],
            release: [0.; RING],
            position: 0,
            attenuation: 1.,
            delta: 0.,
            head: 0,
            count: 0,
        }
    }
    pub(super) fn push(&mut self, input: &[i16]) -> [f32; 2] {
        let voice = if self.frame < BLOCK_FRAMES {
            1. + ((0.25 - 1.) / BLOCK_FRAMES as f32) * self.frame as f32
        } else {
            0.25
        };
        self.frame += 1;
        let pcm = |i: usize| f32::from(input[i]) * (1. / 32_768.);
        let frame = [pcm(0) * voice + pcm(2), pcm(1) * voice + pcm(3)];
        self.limit(frame)
    }
    fn peak_at(&self, p: usize) -> f32 {
        self.buffer[p].abs().max(self.buffer[p + 1].abs())
    }
    fn limit(&mut self, input: [f32; 2]) -> [f32; 2] {
        let p = self.position;
        self.buffer[p..p + 2].copy_from_slice(&input);
        let peak = input[0].abs().max(input[1].abs());
        if peak > LIMIT {
            let target = LIMIT / peak;
            let delta = ((target - self.attenuation) / RING as f32) * 2.;
            let release = (1. - target.min(1.)) / RELEASE_SAMPLES;
            if delta < self.delta {
                self.delta = delta;
                self.peaks[0] = p;
                self.peaks[1] = usize::MAX;
                self.release[0] = release;
                self.head = 0;
                self.count = 1;
            } else {
                for i in 0..self.count {
                    let q = (self.head + i) % RING;
                    let previous = self.peaks[q];
                    let distance = ((RING - previous + p) % RING) / 2;
                    let delta = (target - LIMIT / self.peak_at(previous)) / distance as f32;
                    if delta < self.release[q] {
                        self.release[q] = delta;
                        self.count = i + 1;
                        let tail = (self.head + self.count) % RING;
                        self.peaks[tail] = p;
                        self.release[tail] = release;
                        self.peaks[(tail + 1) % RING] = usize::MAX;
                        self.count += 1;
                        break;
                    }
                }
            }
        }
        let next = (p + 2) % RING;
        self.attenuation += self.delta;
        let mut output = [
            self.buffer[next] * self.attenuation,
            self.buffer[next + 1] * self.attenuation,
        ];
        if next == self.peaks[self.head] {
            self.delta = self.release[self.head];
            self.attenuation = LIMIT / self.peak_at(next);
            self.count -= 1;
            self.peaks[self.head] = usize::MAX;
            self.head = (self.head + 1) % RING;
        }
        if self.attenuation > 1. {
            self.attenuation = 1.;
            self.count = 0;
            self.delta = 0.;
            self.head = 0;
            self.peaks[0] = usize::MAX;
        } else if self.attenuation <= 0. {
            self.attenuation = 1e-6;
            self.delta = 0.999_999 / RELEASE_SAMPLES;
        }
        if self.attenuation != 1. && 1. - self.attenuation < 1e-6 {
            self.attenuation = 1.;
        }
        if self.delta != 0. && self.delta.abs() < 1e-6 {
            self.delta = 0.;
        }
        self.position = next;
        // Native output gain and automatic level compensation cancel here.
        // Preserve float32 multiplication to match its numerical behavior.
        let gain = LIMIT * (1. / LIMIT);
        for sample in &mut output {
            *sample = gain * sample.clamp(-LIMIT, LIMIT);
        }
        output
    }
}
