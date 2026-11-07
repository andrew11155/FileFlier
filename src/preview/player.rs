//! Video playback for Quick Look. ffmpeg decodes the video, scaled down, to raw
//! RGBA frames on a pipe that a reader thread hands over; the UI shows each
//! frame when its time comes. A second ffmpeg plays the sound through
//! PulseAudio (PipeWire's Pulse server on most desktops). Pausing and seeking
//! restart both processes at the new position, which keeps them in step.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::Instant;

/// Largest frame decoded (Quick Look never shows video bigger than this).
const MAX_W: f64 = 1280.0;
const MAX_H: f64 = 720.0;
const MAX_FPS: f64 = 30.0;
/// Hold the first frame this long so the sound (slower to start) lines up.
const AUDIO_LEAD: f64 = 0.12;

#[derive(Clone, Copy)]
pub struct Info {
    /// 0 × 0 for sound only (music, recordings).
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub duration: f64,
    pub has_audio: bool,
}

impl Info {
    pub fn audio_only(&self) -> bool {
        self.w == 0
    }
}

enum Msg {
    Info(Info),
    Frame(Vec<u8>),
    End,
    Failed(String),
}

pub struct Player {
    pub path: PathBuf,
    pub info: Option<Info>,
    pub error: Option<String>,
    pub tex: Option<egui::TextureHandle>,
    pub muted: bool,
    pub ended: bool,
    rx: Receiver<Msg>,
    /// The decoder process, once the background thread has started it.
    video_rx: Receiver<Child>,
    video: Option<ChildGuard>,
    audio: Option<ChildGuard>,
    /// Where this run of the decoder started, in seconds.
    start_pos: f64,
    /// When frame 0 of this run is due (None until it arrives).
    clock: Option<Instant>,
    /// The position while paused.
    paused_at: Option<f64>,
    shown_frames: u64,
    pending: Option<Vec<u8>>,
    /// Paused after a seek: waiting for the one frame to show (the window must be
    /// woken for it, since nothing else is moving).
    awaiting_frame: bool,
}

/// Kills the process when dropped.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn available() -> bool {
    super::have("ffmpeg") && super::have("ffprobe")
}

impl Player {
    pub fn open(path: &Path, muted: bool) -> Self {
        Self::start(path.to_path_buf(), None, 0.0, muted)
    }

    fn start(path: PathBuf, info: Option<Info>, at: f64, muted: bool) -> Self {
        let (tx, rx) = sync_channel(4); // a few frames ahead; ffmpeg waits on the pipe
        let (child_tx, video_rx) = std::sync::mpsc::channel();
        let p = Player {
            path: path.clone(),
            info,
            error: None,
            tex: None,
            muted,
            ended: false,
            rx,
            video_rx,
            video: None,
            audio: None,
            start_pos: at,
            clock: None,
            paused_at: None,
            shown_frames: 0,
            pending: None,
            awaiting_frame: false,
        };
        std::thread::Builder::new().name("video".into()).spawn(move || decode(path, info, at, tx, child_tx)).ok();
        p
    }

    /// Current position in seconds.
    pub fn position(&self) -> f64 {
        if let Some(p) = self.paused_at {
            return p;
        }
        let elapsed = self.clock.map_or(0.0, |c| Instant::now().saturating_duration_since(c).as_secs_f64());
        let pos = self.start_pos + elapsed;
        self.info.map_or(pos, |i| pos.min(i.duration))
    }

    pub fn paused(&self) -> bool {
        self.paused_at.is_some()
    }

    pub fn toggle_pause(&mut self) {
        if self.ended {
            return self.seek(0.0);
        }
        match self.paused_at {
            Some(at) => self.restart(at, false),
            None => {
                let at = self.position();
                self.video = None;
                self.audio = None;
                self.paused_at = Some(at);
            }
        }
    }

    pub fn seek(&mut self, to: f64) {
        let to = self.info.map_or(to, |i| to.clamp(0.0, (i.duration - 0.05).max(0.0)));
        let paused = self.paused_at.is_some() && !self.ended;
        self.restart(to, paused);
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if self.muted {
            self.audio = None;
        } else if !self.paused() && !self.ended && self.clock.is_some() {
            self.audio = play_audio(&self.path, self.info, self.position());
        }
    }

    /// Restart decoding at `at`, keeping the current frame on screen until a new one arrives.
    /// When `paused`, show the frame there and stay paused.
    fn restart(&mut self, at: f64, paused: bool) {
        let tex = self.tex.take();
        *self = Self::start(self.path.clone(), self.info, at, self.muted);
        self.tex = tex;
        if paused {
            self.paused_at = Some(at);
            self.awaiting_frame = true;
        }
    }

    /// Sound only: no frames to wait for, so the clock starts as soon as the length is
    /// known, and it ends when the time is up. Returns false for videos.
    fn update_audio_only(&mut self, ctx: &egui::Context) -> bool {
        while self.info.is_none() {
            match self.rx.try_recv() {
                Ok(Msg::Info(i)) => self.info = Some(i),
                Ok(Msg::Failed(e)) => {
                    self.error = Some(e);
                    return true;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    return true; // still probing (it may turn out to be a video)
                }
                Err(TryRecvError::Disconnected) => return true,
            }
        }
        let Some(info) = self.info.filter(Info::audio_only) else { return false };
        while self.rx.try_recv().is_ok() {}
        self.awaiting_frame = false;
        if self.clock.is_none() {
            self.clock = Some(Instant::now());
            if self.paused_at.is_none() && !self.muted {
                self.audio = play_audio(&self.path, self.info, self.start_pos);
            }
        }
        if !self.paused() && !self.ended && info.duration > 0.0 && self.position() >= info.duration - 0.05 {
            self.ended = true;
            self.audio = None;
        }
        if !self.paused() && !self.ended {
            // Only the time and the bar move.
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        true
    }

    /// Steps `frames` frames forward (or back, if negative) and stays paused there.
    pub fn step_frames(&mut self, frames: i32) {
        let fps = self.info.map_or(25.0, |i| i.fps.max(1.0));
        let at = self.position() + frames as f64 / fps;
        let at = self.info.map_or(at, |i| at.clamp(0.0, (i.duration - 0.5 / fps).max(0.0))).max(0.0);
        self.ended = false;
        self.restart(at, true);
    }

    /// Take in decoded frames and show the one that's due. Call every frame.
    pub fn update(&mut self, ctx: &egui::Context) {
        // The decoder process is handed over so it dies with the player (or on pause).
        if let Ok(child) = self.video_rx.try_recv() {
            self.video = Some(ChildGuard(child));
        }
        if self.update_audio_only(ctx) {
            return;
        }
        loop {
            if self.pending.is_none() {
                match self.rx.try_recv() {
                    Ok(Msg::Info(i)) => {
                        self.info = Some(i);
                        continue;
                    }
                    Ok(Msg::Frame(f)) => self.pending = Some(f),
                    Ok(Msg::End) | Err(TryRecvError::Disconnected) => {
                        // Only a decoder that ran to the end (not one stopped by pause) means the end.
                        if self.video.is_some() && self.clock.is_some() && self.paused_at.is_none() {
                            self.ended = true;
                            self.video = None;
                            self.audio = None;
                        }
                        break;
                    }
                    Ok(Msg::Failed(e)) => {
                        self.error = Some(e);
                        self.awaiting_frame = false;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
            let Some(info) = self.info else { break };
            let first = self.clock.is_none();
            if first {
                // Frame 0 is shown at once; the clock (and the sound) start now.
                self.clock = Some(Instant::now() + std::time::Duration::from_secs_f64(AUDIO_LEAD));
                if self.paused_at.is_none() && !self.muted {
                    self.audio = play_audio(&self.path, self.info, self.start_pos);
                }
            }
            let due = self.shown_frames as f64 / info.fps;
            let now = self.clock.map_or(0.0, |c| Instant::now().saturating_duration_since(c).as_secs_f64());
            if !first && (self.paused_at.is_some() || due > now) {
                break; // not yet (or paused on this frame)
            }
            let px = self.pending.take().unwrap_or_default();
            let img = egui::ColorImage::from_rgba_unmultiplied([info.w as usize, info.h as usize], &px);
            match &mut self.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("video", img, egui::TextureOptions::LINEAR)),
            }
            self.shown_frames += 1;
            if self.paused_at.is_some() {
                // Paused after a seek: show this one frame, then let the decoder go.
                self.video = None;
                self.awaiting_frame = false;
                break;
            }
        }
        if !self.paused() && !self.ended && self.error.is_none() {
            ctx.request_repaint();
        } else if self.awaiting_frame && self.error.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(15));
        }
    }
}

/// Probe (unless known), start ffmpeg, and pass frames to the player.
fn decode(path: PathBuf, info: Option<Info>, at: f64, tx: SyncSender<Msg>, child_tx: std::sync::mpsc::Sender<Child>) {
    let Some(info) = info.or_else(|| probe(&path)) else {
        let _ = tx.send(Msg::Failed("This video can't be played".into()));
        return;
    };
    let _ = tx.send(Msg::Info(info));
    if info.audio_only() {
        return; // the sound is played by `play_audio`; there are no frames
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-ss", &format!("{at:.3}"), "-i"])
        .arg(&path)
        .args(["-an", "-sn", "-vf", &format!("scale={}:{},fps={}", info.w, info.h, info.fps)])
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else {
        let _ = tx.send(Msg::Failed("Couldn't start ffmpeg".into()));
        return;
    };
    let Some(mut out) = child.stdout.take() else { return };
    if let Err(back) = child_tx.send(child) {
        // The player is already gone.
        let _ = ChildGuard(back.0);
        return;
    }
    let size = info.w as usize * info.h as usize * 4;
    loop {
        let mut frame = vec![0u8; size];
        if out.read_exact(&mut frame).is_err() {
            let _ = tx.send(Msg::End);
            return;
        }
        if tx.send(Msg::Frame(frame)).is_err() {
            return; // the player went away
        }
    }
}

/// Output size, frame rate, length and whether there's sound.
fn probe(path: &Path) -> Option<Info> {
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(path);
    let out = super::run_cmd(cmd, std::time::Duration::from_secs(10), None)?;
    let p: serde_json::Value = serde_json::from_slice(&out).ok()?;
    let streams = p["streams"].as_array()?;
    let duration = p["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let has_audio = streams.iter().any(|s| s["codec_type"] == "audio");
    // (A cover picture in a music file doesn't make it a video.)
    let Some(v) = streams.iter().find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1) else {
        return has_audio.then_some(Info { w: 0, h: 0, fps: 25.0, duration, has_audio });
    };
    let (w, h) = (v["width"].as_f64()?, v["height"].as_f64()?);
    let rot = v["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| d["rotation"].as_i64()))
        .or_else(|| v["tags"]["rotate"].as_str().and_then(|r| r.parse().ok()))
        .unwrap_or(0);
    // ffmpeg turns rotated (phone) videos upright as it decodes.
    let (w, h) = if rot.abs() % 180 == 90 { (h, w) } else { (w, h) };
    let (w, h) = fit(w, h);
    let fps = v["avg_frame_rate"]
        .as_str()
        .and_then(|r| r.split_once('/'))
        .and_then(|(n, d)| Some(n.parse::<f64>().ok()? / d.parse::<f64>().ok()?))
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(25.0)
        .min(MAX_FPS);
    Some(Info { w, h, fps, duration, has_audio })
}

/// Scale down to fit `MAX_W` × `MAX_H`, to even sizes (most pixel formats need them).
fn fit(w: f64, h: f64) -> (u32, u32) {
    let s = (MAX_W / w).min(MAX_H / h).min(1.0);
    let even = |x: f64| ((x * s / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

fn play_audio(path: &Path, info: Option<Info>, at: f64) -> Option<ChildGuard> {
    if !info.is_some_and(|i| i.has_audio) {
        return None;
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-ss", &format!("{at:.3}"), "-i"])
        .arg(path)
        .args(["-vn", "-sn", "-f", "pulse", "-buffer_duration", "60", "File Flier"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().ok().map(ChildGuard)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pausing, then stepping frames, shows each frame (needs ffmpeg; skipped without).
    #[test]
    fn steps_frames_while_paused() {
        if !available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("file-flier-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("t.mp4");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=64x48:rate=10", "-t", "3"])
            .arg(&video)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return; // no encoder for it here
        }
        let ctx = egui::Context::default();
        let wait = |p: &mut Player, done: &dyn Fn(&Player) -> bool| {
            let start = Instant::now();
            while !done(p) && start.elapsed().as_secs() < 10 {
                p.update(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        let mut p = Player::open(&video, true);
        wait(&mut p, &|p| p.tex.is_some());
        p.toggle_pause();
        let at = p.position();
        p.step_frames(3);
        assert!(p.awaiting_frame);
        wait(&mut p, &|p| !p.awaiting_frame);
        assert!(!p.awaiting_frame, "the stepped-to frame never came");
        assert!(p.paused());
        assert!((p.position() - (at + 0.3)).abs() < 1e-6);
        p.step_frames(-1);
        wait(&mut p, &|p| !p.awaiting_frame);
        assert!((p.position() - (at + 0.2)).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sound files play with no frames: the clock runs and they end (muted here).
    #[test]
    fn plays_sound_only_files() {
        if !available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("file-flier-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sound = dir.join("t.wav");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=0.6"])
            .arg(&sound)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        let ctx = egui::Context::default();
        let mut p = Player::open(&sound, true);
        let start = Instant::now();
        while !p.ended && start.elapsed().as_secs() < 10 {
            p.update(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(p.info.is_some_and(|i| i.audio_only() && i.has_audio), "{:?}", p.error);
        assert!(p.ended, "never got to the end");
        assert!(p.tex.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fits_and_keeps_sizes_even() {
        assert_eq!(fit(3840.0, 2160.0), (1280, 720));
        assert_eq!(fit(1080.0, 1920.0), (406, 720));
        assert_eq!(fit(641.0, 361.0), (642, 362)); // small videos aren't enlarged, just evened
    }
}
