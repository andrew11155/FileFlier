//! Video (via ffmpeg/ffprobe) and audio (tags, cover art, duration).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use super::{Content, Cx, Loaded, Rgba, format_duration, row};

const FRAME_MAX: u32 = 1600;

// ------------------------------------------------------------------ video

pub fn load_video(path: &Path, cx: &Cx) -> Loaded {
    if !super::have("ffmpeg") {
        return Loaded::new(Content::Icon(Some("Install ffmpeg to see video previews".into())));
    }
    let probe = ffprobe(path);
    let duration = probe.as_ref().and_then(|p| p["format"]["duration"].as_str()?.parse::<f64>().ok());
    let info = probe.as_ref().map(video_rows).unwrap_or_default();
    if cx.cancelled() {
        return Loaded::new(Content::Icon(None));
    }
    let content = match video_frame(path, FRAME_MAX, duration) {
        Some(img) => Content::Image(img),
        None => Content::Icon(Some("No picture in this video".into())),
    };
    Loaded::new(content).with_info(info)
}

/// A representative frame (10% in, to skip black intros).
pub fn video_frame(path: &Path, max: u32, duration: Option<f64>) -> Option<Rgba> {
    if !super::have("ffmpeg") {
        return None;
    }
    let at = duration.map_or(1.0, |d| if d > 3.0 { d * 0.1 } else { 0.0 });
    let grab = |at: f64| {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-v", "error", "-nostdin", "-ss", &format!("{at:.2}"), "-i"]).arg(path).args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale='min(iw,{max})':'min(ih,{max})':force_original_aspect_ratio=decrease"),
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "-",
        ]);
        let png = super::run_cmd(cmd, Duration::from_secs(20), None)?;
        image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()
    };
    let img = grab(at).or_else(|| if at > 0.0 { grab(0.0) } else { None })?;
    Some(Rgba::from_image(img, max))
}

fn ffprobe(path: &Path) -> Option<serde_json::Value> {
    if !super::have("ffprobe") {
        return None;
    }
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(path);
    let out = super::run_cmd(cmd, Duration::from_secs(10), None)?;
    serde_json::from_slice(&out).ok()
}

fn codec_name(c: &str) -> String {
    match c {
        "h264" => "H.264".into(),
        "hevc" => "HEVC (H.265)".into(),
        "av1" => "AV1".into(),
        "vp8" | "vp9" | "aac" | "mp3" | "flac" | "opus" | "ac3" | "eac3" | "dts" | "mpeg4" => c.to_uppercase(),
        "vorbis" => "Vorbis".into(),
        "prores" => "ProRes".into(),
        "mpeg2video" => "MPEG-2".into(),
        "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" => "PCM".into(),
        "alac" => "Apple Lossless".into(),
        other => other.to_string(),
    }
}

fn channels(n: u64) -> String {
    match n {
        1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n} channels"),
    }
}

fn bitrate(bps: f64) -> String {
    if bps >= 1e6 { format!("{:.1} Mbps", bps / 1e6) } else { format!("{:.0} kbps", bps / 1e3) }
}

fn video_rows(p: &serde_json::Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let streams = p["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1);
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    if let Some(v) = video
        && let (Some(w), Some(h)) = (v["width"].as_u64(), v["height"].as_u64())
    {
        let rot = v["side_data_list"]
            .as_array()
            .and_then(|l| l.iter().find_map(|d| d["rotation"].as_i64()))
            .or_else(|| v["tags"]["rotate"].as_str().and_then(|r| r.parse().ok()))
            .unwrap_or(0);
        let (w, h) = if rot.abs() % 180 == 90 { (h, w) } else { (w, h) };
        rows.push(row("Dimensions", format!("{w} × {h}")));
    }
    if let Some(d) = p["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()) {
        rows.push(row("Duration", format_duration(d)));
    }
    if let Some(v) = video {
        let mut parts = vec![codec_name(v["codec_name"].as_str().unwrap_or("?"))];
        if let Some((n, d)) = v["avg_frame_rate"].as_str().and_then(|r| r.split_once('/')) {
            let (n, d): (f64, f64) = (n.parse().unwrap_or(0.0), d.parse().unwrap_or(0.0));
            if d > 0.0 && n > 0.0 {
                let fps = n / d;
                parts.push(if (fps - fps.round()).abs() < 0.01 {
                    format!("{fps:.0} fps")
                } else {
                    format!("{fps:.2} fps")
                });
            }
        }
        rows.push(row("Video", parts.join(", ")));
    }
    if let Some(a) = audio {
        let mut parts = vec![codec_name(a["codec_name"].as_str().unwrap_or("?"))];
        if let Some(c) = a["channels"].as_u64() {
            parts.push(channels(c));
        }
        if let Some(r) = a["sample_rate"].as_str().and_then(|r| r.parse::<f64>().ok()) {
            parts.push(format!("{} kHz", r / 1000.0));
        }
        rows.push(row("Audio", parts.join(", ")));
    }
    if let Some(b) = p["format"]["bit_rate"].as_str().and_then(|b| b.parse::<f64>().ok()) {
        rows.push(row("Bit rate", bitrate(b)));
    }
    let tags = &p["format"]["tags"];
    if let Some(t) = tags["title"].as_str().filter(|t| !t.is_empty()) {
        rows.push(row("Title", t));
    }
    if let Some(t) = tags["creation_time"].as_str() {
        rows.push(row("Recorded", t.get(..16).unwrap_or(t).replace('T', " ")));
    }
    rows
}

// ------------------------------------------------------------------ audio

pub fn load_audio(path: &Path) -> Loaded {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::tag::Accessor;
    let mut rows = Vec::new();
    let mut cover = None;
    match lofty::read_from_path(path) {
        Ok(f) => {
            let tag = f.primary_tag().or_else(|| f.first_tag());
            if let Some(t) = tag {
                for (k, v) in [("Title", t.title()), ("Artist", t.artist()), ("Album", t.album()), ("Genre", t.genre())]
                {
                    if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
                        rows.push(row(k, v.trim()));
                    }
                }
                if let Some(d) = t.date() {
                    rows.push(row("Year", d.year.to_string()));
                }
                if let Some(n) = t.track() {
                    let total = t.track_total().map(|t| format!(" of {t}")).unwrap_or_default();
                    rows.push(row("Track", format!("{n}{total}")));
                }
                cover = t.pictures().first().and_then(|p| image::load_from_memory(p.data()).ok());
            }
            let p = f.properties();
            rows.push(row("Duration", format_duration(p.duration().as_secs_f64())));
            let mut fmt = Vec::new();
            if let Some(b) = p.audio_bitrate().filter(|&b| b > 0) {
                fmt.push(format!("{b} kbps"));
            }
            if let Some(r) = p.sample_rate() {
                fmt.push(format!("{} kHz", r as f64 / 1000.0));
            }
            if let Some(d) = p.bit_depth() {
                fmt.push(format!("{d}-bit"));
            }
            if let Some(c) = p.channels() {
                fmt.push(channels(c as u64));
            }
            if !fmt.is_empty() {
                rows.push(row("Format", fmt.join(", ")));
            }
        }
        Err(_) => {
            // Formats lofty doesn't know (WMA, ...): ask ffprobe.
            if let Some(p) = ffprobe(path) {
                let tags = &p["format"]["tags"];
                for (k, key) in [("Title", "title"), ("Artist", "artist"), ("Album", "album")] {
                    if let Some(v) = tags[key].as_str().or_else(|| tags[key.to_uppercase()].as_str()) {
                        rows.push(row(k, v));
                    }
                }
                rows.extend(video_rows(&p).into_iter().filter(|(k, _)| k != "Dimensions" && k != "Video"));
            }
        }
    }
    let content = match cover {
        Some(img) => Content::Image(Rgba::from_image(img, 1600)),
        None => Content::Icon(None),
    };
    Loaded::new(content).with_info(rows)
}

pub fn audio_cover(path: &Path, max: u32) -> Option<Rgba> {
    use lofty::file::TaggedFileExt;
    let f = lofty::read_from_path(path).ok()?;
    let t = f.primary_tag().or_else(|| f.first_tag())?;
    let img = image::load_from_memory(t.pictures().first()?.data()).ok()?;
    Some(Rgba::from_image(img, max))
}
