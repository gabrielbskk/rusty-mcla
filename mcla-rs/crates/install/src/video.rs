//! Video conversion: port of `tools/convert_video.py`.
//!
//! The disc's Bink movies are turned into `<out>/<name>.mcv` + `<out>/<name>.mp3` for the PC runtime. The `.mcv` container is a list of
//! JPEG frames: `"MCV1"`, u32 width, u32 height, u32 fps * 1000, u32 frame count, (frame count + 1) x u64 offsets, JPEG data (all little-endian).
//! The four Bink audio tracks (front pair, centre, LFE, rear pair) are folded to stereo.

use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::ffmpeg;
use crate::manifest::Movie;
use crate::progress::{Ctx, Stage, Unit};
use mcla_formats::ReadAt;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

fn bad(what: impl Into<String>) -> InstallError {
    InstallError::new(ErrorKind::Convert, what)
}

/// (start, end) of every JPEG image in a concatenated stream.
pub fn split_jpegs(data: &[u8]) -> Result<Vec<(usize, usize)>> {
    let mut frames = Vec::new();
    let n = data.len();
    let mut p = 0usize;
    while p + 4 <= n && data[p] == 0xFF && data[p + 1] == 0xD8 {
        let start = p;
        p += 2;
        // header segments up to the scan
        loop {
            if *data.get(p).ok_or_else(|| bad("truncated JPEG header"))? != 0xFF {
                return Err(bad(format!("bad JPEG marker at {p}")));
            }
            let m = *data.get(p + 1).ok_or_else(|| bad("truncated JPEG header"))?;
            let ln = ((*data.get(p + 2).ok_or_else(|| bad("truncated JPEG header"))? as usize) << 8) | *data.get(p + 3).ok_or_else(|| bad("truncated JPEG header"))? as usize;
            p += 2 + ln;
            if m == 0xDA {
                break;
            }
        }
        // entropy data up to the end marker
        loop {
            let off = data.get(p..).and_then(|s| s.iter().position(|&b| b == 0xFF)).ok_or_else(|| bad("JPEG scan without end marker"))?;
            p += off;
            let m = *data.get(p + 1).ok_or_else(|| bad("JPEG scan without end marker"))?;
            if m == 0 || (0xD0..=0xD7).contains(&m) {
                p += 2;
                continue;
            }
            if m == 0xD9 {
                p += 2;
                break;
            }
            return Err(bad(format!("unexpected marker {m:02x} in scan")));
        }
        frames.push((start, p));
    }
    Ok(frames)
}

/// Width and height from the first frame's SOF0 segment (0, 0 if there is none).
fn frame_size(data: &[u8], first_end: usize) -> (u32, u32) {
    let mut p = 2usize;
    while p + 4 <= first_end.min(data.len()) {
        let m = data[p + 1];
        let ln = ((data[p + 2] as usize) << 8) | data[p + 3] as usize;
        if m == 0xC0 && p + 9 <= data.len() {
            let h = u16::from_be_bytes([data[p + 5], data[p + 6]]) as u32;
            let w = u16::from_be_bytes([data[p + 7], data[p + 8]]) as u32;
            return (w, h);
        }
        p += 2 + ln;
    }
    (0, 0)
}

fn probe_fps(ff: &Path, src: &Path) -> f64 {
    let out = ffmpeg::command(ff).arg("-i").arg(src).stdout(Stdio::null()).stderr(Stdio::piped()).output();
    if let Ok(o) = out {
        let text = String::from_utf8_lossy(&o.stderr).into_owned();
        for tok in text.split(',') {
            let tok = tok.trim();
            if tok.ends_with(" fps") {
                if let Some(v) = tok.split_whitespace().next().and_then(|s| s.parse::<f64>().ok()) {
                    return v;
                }
            }
        }
    }
    30.0
}

fn write_atomic_with(target: &Path, f: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>) -> Result<()> {
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let mut file = std::fs::File::create(&tmp).ctx(tmp.display())?;
    f(&mut file).ctx(tmp.display())?;
    file.flush().ctx(tmp.display())?;
    drop(file);
    std::fs::rename(&tmp, target).ctx(target.display())
}

/// Convert one movie file on disk (`src`) to `<out>/<name>.mcv` and `.mp3`.
pub fn convert_movie(ff: &Path, src: &Path, out: &Path, name: &str, ctx: &Ctx) -> Result<()> {
    ctx.message(Stage::Video, format!("{name}: video"));
    let r = ffmpeg::command(ff)
        .args(["-v", "error", "-i"])
        .arg(src)
        .args(["-an", "-c:v", "mjpeg", "-q:v", "5", "-pix_fmt", "yuvj420p", "-f", "image2pipe", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| bad(format!("run ffmpeg: {e}")))?;
    if !r.status.success() {
        return Err(bad(format!("ffmpeg video: {}", String::from_utf8_lossy(&r.stderr).trim())));
    }
    let data = r.stdout;
    let frames = split_jpegs(&data)?;
    let first = frames.first().ok_or_else(|| bad("the movie has no video frames"))?;
    let (w, h) = frame_size(&data, first.1);
    let fps = probe_fps(ff, src);
    let head = 20 + 8 * (frames.len() as u64 + 1);
    write_atomic_with(&out.join(format!("{name}.mcv")), |f| {
        let mut buf = Vec::with_capacity(head as usize);
        buf.extend_from_slice(b"MCV1");
        for v in [w, h, (fps * 1000.0).round() as u32, frames.len() as u32] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        let mut o = head;
        for (a, b) in &frames {
            buf.extend_from_slice(&o.to_le_bytes());
            o += (b - a) as u64;
        }
        buf.extend_from_slice(&o.to_le_bytes());
        f.write_all(&buf)?;
        for (a, b) in &frames {
            f.write_all(&data[*a..*b])?;
        }
        Ok(())
    })?;
    drop(data);
    ctx.message(Stage::Video, format!("{name}: audio"));
    // tracks: 0 = front L/R, 1 = centre, 2 = LFE, 3 = rear L/R
    let mix = "[0:a:0][0:a:1][0:a:2][0:a:3]amerge=inputs=4,pan=stereo|c0=c0+0.707*c2+0.5*c3+0.6*c4|c1=c1+0.707*c2+0.5*c3+0.6*c5[a]";
    let mp3 = out.join(format!("{name}.mp3"));
    let tmp = out.join(format!("{name}.part.mp3"));
    let mut args: Vec<OsString> = vec!["-v".into(), "error".into(), "-y".into(), "-i".into(), src.into(), "-filter_complex".into(), mix.into(), "-map".into(), "[a]".into(), "-ar".into(), "44100".into(), "-b:a".into(), "192k".into(), (&tmp).into()];
    let (mut good, mut err) = ffmpeg::run(ff, &args);
    if !good {
        // fewer tracks than expected: the first one alone
        args = vec!["-v".into(), "error".into(), "-y".into(), "-i".into(), src.into(), "-map".into(), "0:a:0".into(), "-ar".into(), "44100".into(), "-b:a".into(), "192k".into(), (&tmp).into()];
        (good, err) = ffmpeg::run(ff, &args);
    }
    if !good {
        let _ = std::fs::remove_file(&tmp);
        return Err(bad(format!("ffmpeg audio: {err}")));
    }
    let _ = std::fs::remove_file(&mp3);
    std::fs::rename(&tmp, &mp3).ctx(mp3.display())
}

/// Copy a file of the disc to `dest`.
fn copy_from_disc(src: &impl ReadAt, dest: &Path, ctx: &Ctx) -> Result<()> {
    let mut f = std::fs::File::create(dest).ctx(dest.display())?;
    let len = src.len();
    let mut off = 0u64;
    while off < len {
        ctx.check()?;
        let n = (len - off).min(1 << 24) as usize;
        let chunk = src.read_vec(off, n).map_err(|e| InstallError::new(ErrorKind::Unreadable, format!("{e:#}")))?;
        f.write_all(&chunk).ctx(dest.display())?;
        off += n as u64;
    }
    Ok(())
}

/// Convert the movies listed in the manifest. `open(file)` gives the file on the disc. Finished movies (both outputs present) are skipped.
pub fn convert_all<R: ReadAt>(movies: &[Movie], open: &dyn Fn(&str) -> Result<R>, ff: &Path, out: &Path, ctx: &Ctx) -> Result<()> {
    std::fs::create_dir_all(out).ctx(out.display())?;
    let counter = ctx.counter(Stage::Video, Unit::Items, movies.len() as u64);
    counter.set_message("Converting video");
    for m in movies {
        ctx.check()?;
        if out.join(format!("{}.mcv", m.name)).is_file() && out.join(format!("{}.mp3", m.name)).is_file() {
            counter.add(1)?;
            continue;
        }
        let tmp = out.join(format!("{}.bik", m.name));
        let src = open(&m.file)?;
        copy_from_disc(&src, &tmp, ctx)?;
        let r = convert_movie(ff, &tmp, out, &m.name, ctx);
        let _ = std::fs::remove_file(&tmp);
        r?;
        counter.add(1)?;
    }
    counter.finish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(payload: &[u8]) -> Vec<u8> {
        // SOI, SOF0 (h=2, w=3), SOS header, scan bytes with a stuffed 0xFF, EOI
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 2, 0, 3, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
        v.extend_from_slice(&[0xFF, 0xDA, 0, 4, 1, 2]);
        v.extend_from_slice(payload);
        v.extend_from_slice(&[0xFF, 0x00, 7, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn splits_two_frames_and_reads_size() {
        let mut d = jpeg(&[1, 2, 3]);
        let first_len = d.len();
        d.extend_from_slice(&jpeg(&[9]));
        let f = split_jpegs(&d).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0], (0, first_len));
        assert_eq!(f[1].0, first_len);
        assert_eq!(frame_size(&d, f[0].1), (3, 2));
    }

    #[test]
    fn truncated_stream_is_an_error() {
        let d = jpeg(&[1, 2, 3]);
        assert!(split_jpegs(&d[..d.len() - 6]).is_err());
    }
}
