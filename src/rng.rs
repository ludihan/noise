//! The random numbers noise uses (noise in synths and drums, dither,
//! MultiSynths' and `Yxx`'s chances, humanizing): xorshift, small, fast
//! and the same every time from the same seed.

#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Rng {
    fn step(&mut self) -> u32 {
        // Zero would only ever give zero.
        if self.0 == 0 {
            self.0 = 0x2545_f491;
        }
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in 0..1.
    pub fn unit(&mut self) -> f32 {
        self.step() as f32 / u32::MAX as f32
    }

    /// Uniform in -1..1.
    pub fn next(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}
