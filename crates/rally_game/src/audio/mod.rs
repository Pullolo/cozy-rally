//! Audio facade. Everything is synthesized in code (see `synth/`); this module owns the
//! output device (cpal: WASAPI on Windows, ALSA on Linux) and translates game state into
//! synth commands (see `bridge.rs`).
//!
//! Threading: the game thread sends plain-data `Cmd`s through an mpsc channel (lock-free,
//! non-blocking `try_recv` in the callback); the audio callback owns the `Synth` outright, so
//! there is no shared lock with the real-time thread and it never allocates.
//!
//! Without a usable output device everything degrades to silent no-ops.
//! Debug hook: `COZY_AUDIO_WAV=/path/out.wav` renders the live game audio in real time into a
//! WAV file instead of opening a device (works headless).
mod bridge;
// shared with the offline renderer (examples/audio_render.rs), which uses more of its API
#[allow(dead_code)]
mod synth;

use rally_core::world::WorldEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{LazyLock, Mutex, OnceLock};
use synth::{Cmd, EngineParams, Mood, SfxKind, Synth, UiKind, BLOCK, MAX_CARS};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSound {
    Move,
    Select,
    Back,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Music {
    Menu,
    Race,
    Results,
    None,
}

/// Per-car data the engine/tyre sounds need each frame (legacy entry point, see `update_cars`)
#[derive(Clone, Copy, Debug, Default)]
pub struct CarSound {
    pub rpm: f32,
    pub redline: f32,
    pub throttle: f32,
    pub speed: f32,
    pub skid: f32,
    pub loose_surface: bool,
    pub on_ground: bool,
    /// Distance to the closest local listener (m)
    pub distance: f32,
    /// -1 (left) .. 1 (right) relative to the listener
    pub pan: f32,
    pub is_local_player: bool,
    pub engine_kind: u8,
}

struct Audio {
    tx: Mutex<Sender<Cmd>>,
}

static AUDIO: OnceLock<Audio> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
static BRIDGE: LazyLock<Mutex<bridge::Bridge>> = LazyLock::new(|| Mutex::new(bridge::Bridge::new()));

fn send(cmd: Cmd) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(a) = AUDIO.get() {
        if let Ok(tx) = a.tx.lock() {
            let _ = tx.send(cmd);
        }
    }
}

/// Start audio. Never panics; without a device the game simply runs muted.
pub fn init() {
    if AUDIO.get().is_some() {
        return;
    }
    let (tx, rx) = channel::<Cmd>();
    if AUDIO.set(Audio { tx: Mutex::new(tx) }).is_err() {
        return;
    }
    ENABLED.store(true, Ordering::Relaxed);
    let wav = std::env::var("COZY_AUDIO_WAV").ok().filter(|s| !s.is_empty());
    let spawned = std::thread::Builder::new().name("cozy-audio".into()).spawn(move || {
        let ok = match wav {
            Some(path) => wav_capture(rx, &path),
            None => device::run(rx),
        };
        if !ok {
            ENABLED.store(false, Ordering::Relaxed);
        }
    });
    if spawned.is_err() {
        ENABLED.store(false, Ordering::Relaxed);
    }
}

pub fn ui_sound(s: UiSound) {
    send(Cmd::Ui(match s {
        UiSound::Move => UiKind::Move,
        UiSound::Select => UiKind::Select,
        UiSound::Back => UiKind::Back,
    }));
}

pub fn set_music(m: Music) {
    send(Cmd::Music(match m {
        Music::Menu => Mood::Menu,
        Music::Race => Mood::Race,
        Music::Results => Mood::Results,
        Music::None => Mood::Silent,
    }));
}

/// All 0..1. Defaults: master 0.85, music 0.55, sfx 0.9.
#[allow(dead_code)]
pub fn set_volumes(master: f32, music: f32, sfx: f32) {
    send(Cmd::Volumes { master, music, sfx });
}

/// Legacy/manual engine update (the game uses `race_frame`). Car `i` of the slice is keyed `i`.
#[allow(dead_code)]
pub fn update_cars(cars: &[CarSound], _dt: f32) {
    let mut arr = [EngineParams::default(); MAX_CARS];
    let n = cars.len().min(MAX_CARS);
    for (i, c) in cars.iter().take(n).enumerate() {
        let idle = c.redline * 0.13;
        let ground = if c.on_ground { 1.0 } else { 0.0 };
        let (hard, loose) = if c.loose_surface { (0.0, 1.0) } else { (1.0, 0.0) };
        arr[i] = EngineParams {
            id: i as u32,
            kind: c.engine_kind,
            rpm: c.rpm,
            idle_rpm: idle,
            redline: c.redline,
            throttle: c.throttle,
            load: c.throttle,
            speed: c.speed,
            ground,
            s_hard: hard,
            s_loose: loose,
            slip_lat_hard: c.skid * hard,
            slip_lat_loose: c.skid * loose,
            gain: bridge::attenuation(c.distance),
            pan: c.pan,
            doppler: 1.0,
            muffle: ((c.distance - 10.0) / 140.0).clamp(0.0, 1.0),
            local: c.is_local_player,
            ..Default::default()
        };
    }
    send(Cmd::Cars { n: n as u8, cars: arr, paused: false });
}

/// Legacy/manual one-shot for a world event heard at `distance` metres.
#[allow(dead_code)]
pub fn world_event(e: &WorldEvent, distance: f32) {
    use rally_core::race::RaceEvent;
    let gain = bridge::attenuation(distance);
    let kind = match *e {
        WorldEvent::Collision { strength, other, .. } => SfxKind::Collision { strength, scenery: other.is_none() },
        WorldEvent::Landing { strength, .. } => SfxKind::Landing { strength },
        WorldEvent::Race(RaceEvent::CountdownTick(k)) => SfxKind::CountdownTick(k),
        WorldEvent::Race(RaceEvent::Go) => SfxKind::Go,
        WorldEvent::Race(RaceEvent::Checkpoint { .. }) => SfxKind::Checkpoint,
        WorldEvent::Race(RaceEvent::Lap { .. }) => SfxKind::Lap,
        WorldEvent::Race(RaceEvent::Finish { place, .. }) => SfxKind::Finish { place: place as u32 },
        WorldEvent::Race(RaceEvent::Reset { .. }) => SfxKind::Respawn,
        WorldEvent::Race(RaceEvent::RaceOver) => return,
    };
    send(Cmd::Sfx { kind, gain, pan: 0.0 });
}

/// Fade out and release all engine/tyre voices (leaving a race).
pub fn stop_cars() {
    send(Cmd::StopCars);
}

/// Called once per frame by RaceSession::update (after the world advanced; events are in
/// `session.frame_events`). Drives engine/tyre loops and one-shots.
pub fn race_frame(session: &crate::game::RaceSession, dt: f32) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let listeners: Vec<bridge::Listener> = session.players.iter().map(|p| bridge::Listener { eye: p.cam.eye, target: p.cam.target }).collect();
    let locals: Vec<usize> = session.players.iter().map(|p| p.car).collect();
    if let Ok(mut b) = BRIDGE.lock() {
        b.frame(&session.world, &locals, &listeners, &session.frame_events, session.paused, dt, &mut send);
    }
}

/// Drain pending commands without blocking (bounded so a flood can't stall the callback)
fn drain(rx: &Receiver<Cmd>, synth: &mut Synth) {
    for _ in 0..512 {
        match rx.try_recv() {
            Ok(c) => synth.handle(c),
            Err(_) => break,
        }
    }
}

/// Real-time render into a WAV file (debug hook, no device needed)
fn wav_capture(rx: Receiver<Cmd>, path: &str) -> bool {
    use std::io::{Seek, SeekFrom, Write};
    use std::time::{Duration, Instant};
    let sr = 48000u32;
    let Ok(mut f) = std::fs::File::create(path) else {
        return false;
    };
    let mut synth = Synth::new(sr as f32);
    let _ = f.write_all(&wav_header(sr, 0));
    let start = Instant::now();
    let mut frames: u64 = 0;
    let mut bytes = Vec::with_capacity(BLOCK * 4 * 64);
    let mut last_header = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(10));
        let due = (start.elapsed().as_secs_f64() * sr as f64) as u64;
        bytes.clear();
        while frames < due {
            drain(&rx, &mut synth);
            let n = ((due - frames) as usize).min(BLOCK);
            synth.render_block(n);
            for i in 0..n {
                for v in [synth.out_l[i], synth.out_r[i]] {
                    bytes.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
                }
            }
            frames += n as u64;
        }
        if f.write_all(&bytes).is_err() {
            return true;
        }
        if frames - last_header > sr as u64 / 2 {
            last_header = frames;
            let _ = f.seek(SeekFrom::Start(0));
            let _ = f.write_all(&wav_header(sr, (frames * 4) as u32));
            let _ = f.seek(SeekFrom::End(0));
            let _ = f.flush();
        }
    }
}

fn wav_header(sr: u32, data_bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&2u16.to_le_bytes());
    h[24..28].copy_from_slice(&sr.to_le_bytes());
    h[28..32].copy_from_slice(&(sr * 4).to_le_bytes());
    h[32..34].copy_from_slice(&4u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

mod device {
    use super::{drain, Cmd, Synth, BLOCK};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
    use std::sync::mpsc::Receiver;

    /// Open the default output device and keep the stream alive on this thread.
    /// Returns false (silently) if there is no usable device.
    pub fn run(rx: Receiver<Cmd>) -> bool {
        // Linux: skip the ALSA probe entirely when the machine has no sound card and no
        // sound server (avoids ALSA's error spam on headless boxes / CI).
        #[cfg(target_os = "linux")]
        if !linux_has_audio() {
            return false;
        }
        let host = cpal::default_host();
        let Some(dev) = host.default_output_device() else {
            return false;
        };
        let Ok(supported) = dev.default_output_config() else {
            return false;
        };
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&dev, &config, rx),
            SampleFormat::I16 => build::<i16>(&dev, &config, rx),
            SampleFormat::U16 => build::<u16>(&dev, &config, rx),
            SampleFormat::I32 => build::<i32>(&dev, &config, rx),
            SampleFormat::F64 => build::<f64>(&dev, &config, rx),
            _ => None,
        };
        let Some(stream) = stream else {
            return false;
        };
        if stream.play().is_err() {
            return false;
        }
        // keep the stream (which may be !Send) alive for the life of the program
        loop {
            std::thread::park();
        }
    }

    fn build<T>(dev: &cpal::Device, config: &StreamConfig, rx: Receiver<Cmd>) -> Option<cpal::Stream>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = config.channels.max(1) as usize;
        let mut synth = Synth::new(config.sample_rate.0 as f32);
        let data = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
            drain(&rx, &mut synth);
            let frames = out.len() / channels;
            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(BLOCK);
                synth.render_block(n);
                for i in 0..n {
                    let base = (done + i) * channels;
                    let (l, r) = (synth.out_l[i], synth.out_r[i]);
                    if channels == 1 {
                        out[base] = T::from_sample((l + r) * 0.5);
                    } else {
                        out[base] = T::from_sample(l);
                        out[base + 1] = T::from_sample(r);
                        for c in 2..channels {
                            out[base + c] = T::from_sample(0.0f32);
                        }
                    }
                }
                done += n;
            }
        };
        dev.build_output_stream(config, data, |_err| {}, None).ok()
    }

    #[cfg(target_os = "linux")]
    fn linux_has_audio() -> bool {
        let cards = std::fs::read_to_string("/proc/asound/cards").unwrap_or_default();
        let has_card = !cards.trim().is_empty() && !cards.contains("no soundcards");
        let has_server = std::env::var_os("PULSE_SERVER").is_some()
            || std::env::var_os("PIPEWIRE_REMOTE").is_some()
            || std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| {
                let d = std::path::Path::new(&d);
                d.join("pulse/native").exists() || d.join("pipewire-0").exists()
            });
        has_card || has_server
    }
}
