#[cfg(feature = "audio-wav-dump")]
pub mod wav;

#[cfg(feature = "audio-wav-dump")]
pub use wav::WavAudioSink;

/// One AID block: 8 stereo frames of `(left, right)` s16 samples.
pub type AidBlock = [(i16, i16); 8];

/// One-way audio sink. The emulator pushes stereo frames here on the emu
/// thread; backends route them to a host device, a file, etc.
pub trait AudioSink: Send {
    /// Update the input sample rate of subsequent AID frames.
    fn set_sample_rate(&mut self, _sample_rate: u32) {}

    /// Push one stereo s16 frame.
    fn push_stereo_i16(&mut self, left: i16, right: i16);

    /// Push one full AID block (8 stereo frames). Default impl loops, but
    /// the CPAL sink overrides this to do a single ringbuf push.
    fn push_stereo_block(&mut self, frames: &AidBlock) {
        for &(l, r) in frames {
            self.push_stereo_i16(l, r);
        }
    }

    /// Optional: flush pending state. Most sinks have nothing to do.
    fn flush(&mut self) {}
}

/// No-op sink. The default `System::audio_sink` so headless tools (dspemu,
/// tinybench, tinytracer) work with zero configuration.
pub struct EmptyAudioSink;

impl AudioSink for EmptyAudioSink {
    #[inline(always)]
    fn push_stereo_i16(&mut self, _: i16, _: i16) {}

    #[inline(always)]
    fn push_stereo_block(&mut self, _: &AidBlock) {}
}

pub struct MultiplexAudioSink {
    inner: Vec<Box<dyn AudioSink>>,
}

impl MultiplexAudioSink {
    pub fn new(inner: Vec<Box<dyn AudioSink>>) -> Self {
        Self { inner }
    }
}

impl AudioSink for MultiplexAudioSink {
    fn set_sample_rate(&mut self, sample_rate: u32) {
        for sink in &mut self.inner {
            sink.set_sample_rate(sample_rate);
        }
    }

    #[inline(always)]
    fn push_stereo_i16(&mut self, left: i16, right: i16) {
        for sink in &mut self.inner {
            sink.push_stereo_i16(left, right);
        }
    }

    #[inline(always)]
    fn push_stereo_block(&mut self, frames: &AidBlock) {
        for sink in &mut self.inner {
            sink.push_stereo_block(frames);
        }
    }

    fn flush(&mut self) {
        for sink in &mut self.inner {
            sink.flush();
        }
    }
}

/// Decode a 32-byte AID DMA block into 8 stereo s16 frames.
#[inline(always)]
pub fn decode_aid_block(src: &[u8]) -> AidBlock {
    debug_assert!(src.len() >= 32);

    let mut out = [(0i16, 0i16); 8];
    for i in 0..8 {
        let off = i * 4;
        let r = i16::from_be_bytes([src[off], src[off + 1]]);
        let l = i16::from_be_bytes([src[off + 2], src[off + 3]]);
        out[i] = (l, r);
    }

    out
}
