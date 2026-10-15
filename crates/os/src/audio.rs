//! Lecture mp3 : décodage dans un thread dédié, sortie SDL sans entrée-sortie.
//!
//! Le rappel audio de SDL ne fait que vider un tampon circulaire d'une demi-
//! seconde. Toute lecture sur la carte SD (ouverture, recherche, décodage) se
//! fait dans le thread de décodage, si bien qu'un fichier lent ou corrompu ne
//! peut ni bloquer le son ni geler l'interface, contrairement à Telmi OS qui
//! parcourait les mp3 à débit variable en gardant le verrou audio.
//!
//! Chaque commande (lecture, recherche, arrêt) incrémente une génération : le
//! thread de décodage ignore ses échantillons dès qu'ils deviennent périmés.

use std::collections::VecDeque;
use std::ffi::{c_int, c_void};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{Limit, MetadataOptions};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::log;

pub const SAMPLE_RATE: u32 = 44_100;
/// Tampon d'une demi-seconde, en échantillons stéréo entrelacés.
const RING_SAMPLES: usize = SAMPLE_RATE as usize;

#[derive(Default)]
struct State {
    samples: VecDeque<i16>,
    generation: u64,
    playing: bool,
    paused: bool,
    /// Le décodeur a atteint la fin du fichier (ou une erreur) pour cette génération.
    ended: bool,
    finished: bool,
    /// Position de départ de la génération, en secondes.
    base: f64,
    /// Trames stéréo envoyées à la sortie depuis le début de la génération.
    consumed: u64,
    duration: Option<f64>,
}

struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Un panic ailleurs ne doit pas couper le son : on reprend l'état tel quel.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

enum Command {
    Play { path: PathBuf, start: f64, generation: u64 },
    Seek { time: f64, generation: u64 },
    Stop,
}

pub struct Audio {
    shared: Arc<Shared>,
    commands: Sender<Command>,
}

impl Audio {
    /// Démarre le thread de décodage. Le rappel SDL s'obtient avec [`Audio::callback`].
    pub fn new() -> Audio {
        let shared = Arc::new(Shared {
            state: Mutex::new(State { finished: true, ..State::default() }),
        });
        let (commands, receiver) = mpsc::channel();
        let worker = shared.clone();
        thread::Builder::new()
            .name("audio-decoder".into())
            .spawn(move || decoder_thread(&worker, &receiver))
            .expect("création du thread audio");
        Audio { shared, commands }
    }

    /// Rappel et pointeur à donner à `SDL_OpenAudio`. Le pointeur reste valide
    /// tant que `self` existe.
    pub fn callback(&self) -> (crate::sdl::AudioCallback, *mut c_void) {
        (audio_callback, Arc::as_ptr(&self.shared) as *mut c_void)
    }

    pub fn play(&self, path: &Path, start: f64) {
        let generation = {
            let mut s = self.shared.lock();
            s.generation += 1;
            s.samples.clear();
            s.playing = true;
            s.paused = false;
            s.ended = false;
            s.finished = false;
            s.base = start.max(0.0);
            s.consumed = 0;
            s.duration = None;
            s.generation
        };
        let _ = self.commands.send(Command::Play { path: path.to_path_buf(), start, generation });
    }

    pub fn stop(&self) {
        {
            let mut s = self.shared.lock();
            s.generation += 1;
            s.samples.clear();
            s.playing = false;
            s.paused = false;
            s.ended = true;
            s.finished = true;
            s.base = 0.0;
            s.consumed = 0;
            s.duration = None;
        }
        let _ = self.commands.send(Command::Stop);
    }

    pub fn seek(&self, time: f64) {
        let (time, generation) = {
            let mut s = self.shared.lock();
            if !s.playing || s.finished {
                return;
            }
            let time = match s.duration {
                Some(d) => time.clamp(0.0, d),
                None => time.max(0.0),
            };
            s.generation += 1;
            s.samples.clear();
            s.ended = false;
            s.base = time;
            s.consumed = 0;
            (time, s.generation)
        };
        let _ = self.commands.send(Command::Seek { time, generation });
    }

    pub fn pause(&self) {
        let mut s = self.shared.lock();
        if s.playing && !s.finished {
            s.paused = true;
        }
    }

    pub fn resume(&self) {
        self.shared.lock().paused = false;
    }

    pub fn is_finished(&self) -> bool {
        self.shared.lock().finished
    }

    pub fn is_playing(&self) -> bool {
        let s = self.shared.lock();
        s.playing && !s.finished
    }

    pub fn is_paused(&self) -> bool {
        self.shared.lock().paused
    }

    pub fn position(&self) -> f64 {
        let s = self.shared.lock();
        s.base + s.consumed as f64 / SAMPLE_RATE as f64
    }

    pub fn duration(&self) -> Option<f64> {
        self.shared.lock().duration
    }
}

extern "C" fn audio_callback(userdata: *mut c_void, stream: *mut u8, len: c_int) {
    // SAFETY: SDL fournit un tampon de `len` octets aligné pour des i16 ;
    // `userdata` pointe vers le `Shared` gardé vivant par `Audio`.
    let (shared, out) = unsafe {
        (
            &*(userdata as *const Shared),
            std::slice::from_raw_parts_mut(stream as *mut i16, len.max(0) as usize / 2),
        )
    };
    let mut s = shared.lock();
    if !s.playing || s.paused || s.finished {
        out.fill(0);
        return;
    }
    let available = s.samples.len().min(out.len()) & !1;
    for (o, v) in out.iter_mut().zip(s.samples.drain(..available)) {
        *o = v;
    }
    out[available..].fill(0);
    s.consumed += (available / 2) as u64;
    if s.ended && s.samples.is_empty() {
        s.finished = true;
    }
}

/// Flux mp3 ouvert, avec son rééchantillonneur.
struct Stream {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    buffer: Option<SampleBuffer<i16>>,
    resampler: Resampler,
    generation: u64,
}

fn decoder_thread(shared: &Shared, commands: &Receiver<Command>) {
    let mut stream: Option<Stream> = None;
    let mut current: Option<PathBuf> = None;
    loop {
        let has_room = stream.is_some() && shared.lock().samples.len() < RING_SAMPLES;
        let command = if has_room {
            commands.try_recv().ok()
        } else if stream.is_some() {
            match commands.recv_timeout(Duration::from_millis(15)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match commands.recv() {
                Ok(c) => Some(c),
                Err(_) => return,
            }
        };

        let idle = command.is_none();
        match command {
            Some(Command::Play { path, start, generation }) => {
                stream = start_stream(shared, &path, start, generation);
                current = Some(path);
            }
            Some(Command::Seek { time, generation }) => match (stream.as_mut(), current.as_ref()) {
                (Some(st), _) => {
                    st.generation = generation;
                    let actual = seek(st, time);
                    let mut s = shared.lock();
                    if s.generation == generation {
                        if let Some(actual) = actual {
                            s.base = actual;
                        }
                    }
                }
                // Flux déjà terminé (fin du fichier) : on le rouvre à la bonne position.
                (None, Some(path)) => stream = start_stream(shared, &path.clone(), time, generation),
                (None, None) => end_generation(shared, generation),
            },
            Some(Command::Stop) => {
                stream = None;
                current = None;
            }
            None => {}
        }

        if has_room && idle {
            if let Some(st) = stream.as_mut() {
                if !decode_packet(st, shared) {
                    end_generation(shared, st.generation);
                    stream = None;
                }
            }
        }
    }
}

/// Ouvre `path` à la position `start` et publie la durée et la position réelles.
fn start_stream(shared: &Shared, path: &Path, start: f64, generation: u64) -> Option<Stream> {
    let mut stream = match open(path, generation) {
        Ok(stream) => stream,
        Err(e) => {
            log!("audio {} : {e}", path.display());
            end_generation(shared, generation);
            return None;
        }
    };
    let duration = probe_duration(&stream, path);
    // Recherche impossible : on repart du début plutôt que d'afficher une fausse position.
    let actual = if start > 0.0 { seek(&mut stream, start).unwrap_or(0.0) } else { 0.0 };
    let mut s = shared.lock();
    if s.generation == generation {
        s.duration = duration;
        s.base = actual;
    }
    Some(stream)
}

fn end_generation(shared: &Shared, generation: u64) {
    let mut s = shared.lock();
    if s.generation == generation {
        s.ended = true;
        if s.samples.is_empty() {
            s.finished = true;
        }
    }
}

fn open(path: &Path, generation: u64) -> Result<Stream, SymphoniaError> {
    let file = File::open(path)?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    // Les pochettes intégrées aux tags ID3 ne sont jamais chargées.
    let metadata = MetadataOptions {
        limit_metadata_bytes: Limit::Maximum(64 * 1024),
        limit_visual_bytes: Limit::Maximum(0),
    };
    let probed = symphonia::default::get_probe().format(&hint, source, &FormatOptions::default(), &metadata)?;
    let format = probed.format;
    let track = format
        .default_track()
        .ok_or(SymphoniaError::Unsupported("aucune piste audio"))?;
    let decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let track_id = track.id;
    Ok(Stream { format, decoder, track: track_id, buffer: None, resampler: Resampler::default(), generation })
}

fn probe_duration(stream: &Stream, path: &Path) -> Option<f64> {
    let params = stream.format.tracks().iter().find(|t| t.id == stream.track)?.codec_params.clone();
    match (params.n_frames, params.sample_rate) {
        (Some(frames), Some(rate)) if frames > 0 && rate > 0 => Some(frames as f64 / rate as f64),
        _ => estimate_cbr_duration(path),
    }
}

/// Recherche approximative ; renvoie la position atteinte en secondes.
fn seek(stream: &mut Stream, time: f64) -> Option<f64> {
    let target = SeekTo::Time { time: Time::from(time.max(0.0)), track_id: Some(stream.track) };
    let seeked = stream.format.seek(SeekMode::Coarse, target).ok()?;
    stream.decoder.reset();
    stream.resampler = Resampler::default();
    let time_base = stream.format.tracks().iter().find(|t| t.id == stream.track)?.codec_params.time_base?;
    let t = time_base.calc_time(seeked.actual_ts);
    Some(t.seconds as f64 + t.frac)
}

/// Décode un paquet et pousse les échantillons. `false` en fin de flux.
fn decode_packet(stream: &mut Stream, shared: &Shared) -> bool {
    let packet = match stream.format.next_packet() {
        Ok(p) => p,
        Err(SymphoniaError::ResetRequired) => {
            stream.decoder.reset();
            return true;
        }
        Err(_) => return false,
    };
    if packet.track_id() != stream.track {
        return true;
    }
    let decoded = match stream.decoder.decode(&packet) {
        Ok(d) => d,
        // Trame corrompue : on la saute, comme mpg123.
        Err(SymphoniaError::DecodeError(_)) => return true,
        Err(_) => return false,
    };
    let spec = *decoded.spec();
    let frames = decoded.capacity() as u64;
    if stream.buffer.as_ref().is_none_or(|b| b.capacity() < decoded.capacity() * spec.channels.count()) {
        stream.buffer = Some(SampleBuffer::new(frames, spec));
    }
    let Some(buffer) = stream.buffer.as_mut() else { return false };
    buffer.copy_interleaved_ref(decoded);
    let channels = spec.channels.count().max(1);
    let mut out = Vec::with_capacity(buffer.samples().len() / channels * 2 + 4);
    stream.resampler.push(buffer.samples(), channels, spec.rate, &mut out);

    let mut s = shared.lock();
    if s.generation == stream.generation {
        s.samples.extend(out);
    }
    true
}

/// Conversion vers 44,1 kHz stéréo par interpolation linéaire.
#[derive(Default)]
struct Resampler {
    previous: [i16; 2],
    phase: f64,
    primed: bool,
}

impl Resampler {
    fn push(&mut self, samples: &[i16], channels: usize, rate: u32, out: &mut Vec<i16>) {
        let step = rate.max(1) as f64 / SAMPLE_RATE as f64;
        for frame in samples.chunks_exact(channels) {
            let current = [frame[0], frame[if channels > 1 { 1 } else { 0 }]];
            if rate == SAMPLE_RATE {
                out.extend_from_slice(&current);
                continue;
            }
            if !self.primed {
                self.previous = current;
                self.primed = true;
            }
            while self.phase < 1.0 {
                for (&a, &b) in self.previous.iter().zip(&current) {
                    let (a, b) = (a as f64, b as f64);
                    out.push((a + (b - a) * self.phase) as i16);
                }
                self.phase += step;
            }
            self.phase -= 1.0;
            self.previous = current;
        }
    }
}

const MPEG1_L3_KBPS: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const MPEG2_L3_KBPS: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

fn frame_bitrate(data: &[u8]) -> Option<u32> {
    data.windows(3).find_map(|w| {
        // Synchronisation sur 11 bits, pour accepter aussi MPEG 2.5.
        if w[0] != 0xFF || w[1] & 0xE0 != 0xE0 {
            return None;
        }
        let version = (w[1] >> 3) & 0x03;
        let layer = (w[1] >> 1) & 0x03;
        let index = ((w[2] >> 4) & 0x0F) as usize;
        if layer != 1 || index == 0 || index == 15 || (w[2] >> 2) & 0x03 == 3 {
            return None;
        }
        // 3 : MPEG 1, 2 : MPEG 2, 0 : MPEG 2.5, 1 : réservé. Telmi OS confondait
        // 1 et 2, si bien que les mp3 MPEG 2 n'avaient jamais d'estimation.
        match version {
            3 => Some(MPEG1_L3_KBPS[index]),
            2 | 0 => Some(MPEG2_L3_KBPS[index]),
            _ => None,
        }
    })
}

/// Durée d'un mp3 à débit constant sans en-tête Xing, comme Telmi OS :
/// taille utile divisée par le débit, si le début et le milieu concordent.
fn estimate_cbr_duration(path: &Path) -> Option<f64> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut head = [0u8; 4096];
    let n = file.read(&mut head).ok()?;
    let mut start = 0u64;
    if n >= 10 && &head[..3] == b"ID3" {
        start = 10 + head[6..10].iter().fold(0u64, |acc, &b| acc << 7 | (b & 0x7F) as u64);
        if head[5] & 0x10 != 0 {
            start += 10;
        }
    }
    if start >= size {
        return None;
    }
    let mut first = [0u8; 4096];
    file.seek(SeekFrom::Start(start)).ok()?;
    let n_first = file.read(&mut first).ok()?;
    let middle_pos = start + (size - start) / 2;
    let mut middle = [0u8; 4096];
    file.seek(SeekFrom::Start(middle_pos)).ok()?;
    let n_middle = file.read(&mut middle).ok()?;
    let rate = frame_bitrate(&first[..n_first])?;
    (frame_bitrate(&middle[..n_middle]) == Some(rate))
        .then(|| (size - start) as f64 * 8.0 / (rate as f64 * 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_mono_22k_to_stereo_44k() {
        let mut r = Resampler::default();
        let mut out = Vec::new();
        r.push(&[0, 1000, 2000, 3000], 1, 22_050, &mut out);
        assert_eq!(out.len() % 2, 0);
        assert!((6..=8).contains(&(out.len() / 2)), "{}", out.len());
        assert!(out.chunks(2).all(|f| f[0] == f[1]));
    }

    #[test]
    fn passes_44k_stereo_through() {
        let mut r = Resampler::default();
        let mut out = Vec::new();
        r.push(&[1, 2, 3, 4], 2, SAMPLE_RATE, &mut out);
        assert_eq!(out, [1, 2, 3, 4]);
    }

    #[test]
    fn reads_frame_bitrate() {
        assert_eq!(frame_bitrate(&[0, 0xFF, 0xFB, 0x90, 0]), Some(128));
        assert_eq!(frame_bitrate(&[0xFF, 0xF3, 0x50]), Some(40), "MPEG 2");
        assert_eq!(frame_bitrate(&[0xFF, 0xE3, 0x50]), Some(40), "MPEG 2.5");
        assert_eq!(frame_bitrate(&[0xFF, 0xEB, 0x50]), None, "version réservée");
        assert_eq!(frame_bitrate(&[1, 2, 3, 4]), None);
    }

    /// `TELMI_STORIES=~/.telmi/stories cargo test --release -p telmi-os real_mp3 -- --nocapture`
    #[test]
    fn real_mp3_duration_seek_and_speed() {
        let Some(root) = std::env::var_os("TELMI_STORIES") else { return };
        let mut files: Vec<PathBuf> = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .filter_map(|d| std::fs::read_dir(d.path().join("audios")).ok())
            .flat_map(|dir| dir.flatten().take(400).map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "mp3"))
            .collect();
        files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map_or(0, |m| m.len())));
        let shared = Shared { state: Mutex::new(State::default()) };
        let (mut no_duration, mut bad_seek) = (0, 0);
        for path in files.iter().take(60) {
            let mut stream = open(path, 0).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let Some(duration) = probe_duration(&stream, path) else {
                no_duration += 1;
                continue;
            };
            let target = duration / 2.0;
            match seek(&mut stream, target) {
                Some(actual) if (actual - target).abs() < 2.0 => {}
                other => {
                    bad_seek += 1;
                    println!("recherche {target:.1}s → {other:?} : {}", path.display());
                }
            }
            for _ in 0..20 {
                assert!(decode_packet(&mut stream, &shared));
            }
        }
        let biggest = &files[0];
        let mut stream = open(biggest, 0).unwrap();
        let started = std::time::Instant::now();
        let mut samples = 0usize;
        while decode_packet(&mut stream, &shared) {
            let mut s = shared.lock();
            samples += s.samples.len();
            s.samples.clear();
        }
        let audio_secs = samples as f64 / 2.0 / SAMPLE_RATE as f64;
        println!(
            "{} fichiers, {no_duration} sans durée, {bad_seek} recherches imprécises ; \
             décodage {:.0}× le temps réel ({:.0} s d'audio)",
            files.len().min(60),
            audio_secs / started.elapsed().as_secs_f64(),
            audio_secs
        );
        assert_eq!(bad_seek, 0);
    }
}
