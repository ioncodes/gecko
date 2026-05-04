use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use crate::audio::{AidBlock, AudioSink};

pub struct WavAudioSink {
    path: PathBuf,
    pending_rate: u32,
    writer: Option<hound::WavWriter<BufWriter<File>>>,
}

impl WavAudioSink {
    pub fn create(path: impl Into<PathBuf>, emulated_rate: u32) -> Self {
        let path = path.into();
        tracing::info!(
            path = %path.display(),
            sample_rate = emulated_rate,
            "Configured audio WAV dump"
        );

        Self {
            path,
            pending_rate: emulated_rate,
            writer: None,
        }
    }

    fn ensure_open(&mut self) -> &mut hound::WavWriter<BufWriter<File>> {
        if self.writer.is_none() {
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate: self.pending_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let writer = hound::WavWriter::create(&self.path, spec).expect("failed to open WAV file for audio dump");

            tracing::info!(
                path = %self.path.display(),
                sample_rate = self.pending_rate,
                "Opened audio WAV dump"
            );

            self.writer = Some(writer);
        }

        self.writer.as_mut().unwrap()
    }
}

impl AudioSink for WavAudioSink {
    fn set_sample_rate(&mut self, sample_rate: u32) {
        if self.pending_rate == sample_rate {
            return;
        }

        if self.writer.is_some() {
            tracing::warn!(
                old_sample_rate = self.pending_rate,
                new_sample_rate = sample_rate,
                "WAV dump sample-rate change after first audio block; keeping existing WAV rate"
            );
            return;
        }

        tracing::info!(
            old_sample_rate = self.pending_rate,
            new_sample_rate = sample_rate,
            "Updated pending audio WAV dump sample rate"
        );
        self.pending_rate = sample_rate;
    }

    fn push_stereo_i16(&mut self, left: i16, right: i16) {
        let w = self.ensure_open();
        let _ = w.write_sample(left);
        let _ = w.write_sample(right);
    }

    fn push_stereo_block(&mut self, frames: &AidBlock) {
        let w = self.ensure_open();
        for &(l, r) in frames {
            let _ = w.write_sample(l);
            let _ = w.write_sample(r);
        }
    }

    fn flush(&mut self) {
        if let Some(w) = self.writer.as_mut() {
            let _ = w.flush();
        }
    }
}

impl Drop for WavAudioSink {
    fn drop(&mut self) {
        if let Some(w) = self.writer.take() {
            if let Err(err) = w.finalize() {
                tracing::warn!(?err, "failed to finalize WAV dump");
            }
        }
    }
}
