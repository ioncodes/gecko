use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig, SupportedStreamConfig};
use gecko::audio::{AidBlock, AudioSink};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

type Frame = (i16, i16);

struct Resampler {
    consumer: ringbuf::HeapCons<Frame>,
    input_rate: Arc<AtomicU32>,
    host_rate: u32,
    prev: Frame,
    next: Frame,
    phase: f64,
    primed: bool,
}

impl Resampler {
    fn new(consumer: ringbuf::HeapCons<Frame>, input_rate: Arc<AtomicU32>, host_rate: u32) -> Self {
        Self {
            consumer,
            input_rate,
            host_rate,
            prev: (0, 0),
            next: (0, 0),
            phase: 0.0,
            primed: false,
        }
    }

    #[inline]
    fn next_frame(&mut self) -> Frame {
        if self.host_rate == 0 {
            return (0, 0);
        }

        let input_rate_u32 = self.input_rate.load(Ordering::Relaxed).max(1);
        if input_rate_u32 == self.host_rate {
            self.primed = false;
            return self.consumer.try_pop().unwrap_or((0, 0));
        }

        if !self.primed {
            let Some(prev) = self.consumer.try_pop() else {
                return (0, 0);
            };
            self.prev = prev;
            self.next = self.consumer.try_pop().unwrap_or(prev);
            self.phase = 0.0;
            self.primed = true;
        }

        let out = lerp_frame(self.prev, self.next, self.phase);
        let input_rate = input_rate_u32 as f64;
        self.phase += input_rate / self.host_rate as f64;

        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.prev = self.next;
            let Some(next) = self.consumer.try_pop() else {
                self.primed = false;
                break;
            };
            self.next = next;
        }

        out
    }
}

#[inline]
fn lerp_frame(a: Frame, b: Frame, t: f64) -> Frame {
    let l = a.0 as f64 + (b.0 as f64 - a.0 as f64) * t;
    let r = a.1 as f64 + (b.1 as f64 - a.1 as f64) * t;
    (l.round() as i16, r.round() as i16)
}

pub struct CpalBackend {
    pub sink: CpalAudioSink,
    pub stream: cpal::Stream,
}

pub struct CpalAudioSink {
    producer: ringbuf::HeapProd<Frame>,
    overflow_counter: Arc<AtomicU64>,
    sample_rate: Arc<AtomicU32>,
}

impl AudioSink for CpalAudioSink {
    fn set_sample_rate(&mut self, sample_rate: u32) {
        let old = self.sample_rate.swap(sample_rate, Ordering::Relaxed);
        if old != sample_rate {
            tracing::info!(
                old_sample_rate = old,
                new_sample_rate = sample_rate,
                "Updated CPAL input sample rate"
            );
        }
    }

    #[inline(always)]
    fn push_stereo_i16(&mut self, left: i16, right: i16) {
        if self.producer.try_push((left, right)).is_err() {
            self.overflow_counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[inline(always)]
    fn push_stereo_block(&mut self, frames: &AidBlock) {
        let pushed = self.producer.push_slice(frames);
        if pushed < frames.len() {
            let dropped = (frames.len() - pushed) as u64;
            self.overflow_counter.fetch_add(dropped, Ordering::Relaxed);
        }
    }
}

pub fn open(emulated_rate: u32) -> Result<CpalBackend, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output audio device".to_string())?;

    let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());
    let supported = pick_supported_config(&device, emulated_rate)?;
    let sample_format = supported.sample_format();
    let host_rate = supported.sample_rate();
    let host_channels = supported.channels() as usize;
    let config: StreamConfig = supported.into();

    if host_rate != emulated_rate {
        tracing::warn!(
            host_rate,
            emulated_rate,
            "Audio device does not support the emulated sample rate; resampling live audio"
        );
    }

    let cap_frames = (emulated_rate.max(host_rate) as usize / 2).max(1024);
    let rb = HeapRb::<Frame>::new(cap_frames);
    let (producer, consumer) = rb.split();
    let overflow_counter = Arc::new(AtomicU64::new(0));
    let input_rate = Arc::new(AtomicU32::new(emulated_rate));
    let mut resampler = Resampler::new(consumer, input_rate.clone(), host_rate);

    tracing::info!(
        device = %device_name,
        host_rate,
        host_channels,
        emulated_rate,
        ?sample_format,
        ringbuf_capacity_frames = cap_frames,
        "Opened CPAL output"
    );

    let err_fn = |err| tracing::error!(?err, "CPAL stream error");

    let stream = match sample_format {
        SampleFormat::F32 => device
            .build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    write_f32(&mut resampler, out, host_channels);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_output_stream f32: {e}"))?,
        SampleFormat::I16 => device
            .build_output_stream(
                &config,
                move |out: &mut [i16], _| {
                    write_i16(&mut resampler, out, host_channels);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_output_stream i16: {e}"))?,
        SampleFormat::U16 => device
            .build_output_stream(
                &config,
                move |out: &mut [u16], _| {
                    write_u16(&mut resampler, out, host_channels);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_output_stream u16: {e}"))?,
        SampleFormat::U8 => device
            .build_output_stream(
                &config,
                move |out: &mut [u8], _| {
                    write_u8(&mut resampler, out, host_channels);
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_output_stream u8: {e}"))?,
        other => return Err(format!("unsupported CPAL sample format: {other:?}")),
    };

    stream.play().map_err(|e| format!("stream.play: {e}"))?;

    Ok(CpalBackend {
        sink: CpalAudioSink {
            producer,
            overflow_counter,
            sample_rate: input_rate,
        },
        stream,
    })
}

fn config_score(cfg: &SupportedStreamConfig) -> u32 {
    let fmt_score = match cfg.sample_format() {
        SampleFormat::F32 => 0,
        SampleFormat::I16 => 1,
        SampleFormat::U16 => 2,
        SampleFormat::U8 => 3,
        _ => 1000,
    };
    let chan_score: u32 = if cfg.channels() == 2 { 0 } else { 10 };
    fmt_score + chan_score
}

fn pick_supported_config(device: &cpal::Device, desired_rate: u32) -> Result<SupportedStreamConfig, String> {
    let configs = device
        .supported_output_configs()
        .map_err(|e| format!("supported_output_configs: {e}"))?;

    let target_rate: SampleRate = desired_rate;
    let mut best_at_rate: Option<SupportedStreamConfig> = None;

    for range in configs {
        if range.min_sample_rate() <= target_rate && range.max_sample_rate() >= target_rate {
            let candidate = range.with_sample_rate(target_rate);
            match &best_at_rate {
                None => best_at_rate = Some(candidate),
                Some(prev) if config_score(&candidate) < config_score(prev) => {
                    best_at_rate = Some(candidate);
                }
                _ => {}
            }
        }
    }

    if let Some(cfg) = best_at_rate {
        return Ok(cfg);
    }

    device
        .default_output_config()
        .map_err(|e| format!("default_output_config: {e}"))
}

#[inline]
fn write_i16(resampler: &mut Resampler, out: &mut [i16], host_channels: usize) {
    if host_channels == 0 {
        return;
    }

    for slot in out.chunks_mut(host_channels) {
        let (l, r) = resampler.next_frame();

        if !slot.is_empty() {
            slot[0] = l;
        }

        if slot.len() >= 2 {
            slot[1] = r;
        }

        for s in slot.iter_mut().skip(2) {
            *s = 0;
        }
    }
}

#[inline]
fn write_f32(resampler: &mut Resampler, out: &mut [f32], host_channels: usize) {
    if host_channels == 0 {
        return;
    }

    let scale = 1.0 / 32768.0;
    for slot in out.chunks_mut(host_channels) {
        let (l, r) = resampler.next_frame();

        if !slot.is_empty() {
            slot[0] = l as f32 * scale;
        }

        if slot.len() >= 2 {
            slot[1] = r as f32 * scale;
        }

        for s in slot.iter_mut().skip(2) {
            *s = 0.0;
        }
    }
}

#[inline]
fn write_u16(resampler: &mut Resampler, out: &mut [u16], host_channels: usize) {
    if host_channels == 0 {
        return;
    }

    for slot in out.chunks_mut(host_channels) {
        let (l, r) = resampler.next_frame();
        let lu = (l as u16) ^ 0x8000;
        let ru = (r as u16) ^ 0x8000;

        if !slot.is_empty() {
            slot[0] = lu;
        }

        if slot.len() >= 2 {
            slot[1] = ru;
        }

        for s in slot.iter_mut().skip(2) {
            *s = 0x8000;
        }
    }
}

#[inline]
fn write_u8(resampler: &mut Resampler, out: &mut [u8], host_channels: usize) {
    if host_channels == 0 {
        return;
    }

    for slot in out.chunks_mut(host_channels) {
        let (l, r) = resampler.next_frame();
        let lu = ((l >> 8) as u8) ^ 0x80;
        let ru = ((r >> 8) as u8) ^ 0x80;

        if !slot.is_empty() {
            slot[0] = lu;
        }

        if slot.len() >= 2 {
            slot[1] = ru;
        }

        for s in slot.iter_mut().skip(2) {
            *s = 0x80;
        }
    }
}
