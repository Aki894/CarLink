// SPDX-License-Identifier: MIT
//! Explicit opt-in, bounded, pre-encoded lab media; no capture or encoder.
use crate::carlife::{Channel, Message, VideoConfig};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const MAX_VIDEO: usize = 16 * 1024 * 1024;
const MAX_FRAME: usize = 512 * 1024;
const MAX_AUDIO: usize = 2 * 1024 * 1024;
const AUDIO_CHUNK: usize = 44100 / 50 * 4;

struct Frame { data: Vec<u8> }

pub struct TestMedia {
    config: VideoConfig,
    fps: u32,
    frames: Vec<Frame>,
    parameter_sets: Vec<u8>,
    pcm: Vec<u8>,
    video_playing: bool,
    audio_playing: bool,
    first_video: bool,
    frame: usize,
    audio_offset: usize,
    video_due: Duration,
    audio_due: Duration,
    last_poll: Duration,
}

fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit as u64 {
        return Err(format!("{} exceeds asset size limit", path.display()));
    }
    let mut data = Vec::new();
    (&mut file).take(limit as u64 + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    if data.len() > limit { return Err("asset grew beyond size limit".into()); }
    Ok(data)
}

fn config(bytes: &[u8]) -> Result<VideoConfig, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "video.conf must be UTF-8")?;
    let values = text.split_whitespace().map(str::parse::<u32>).collect::<Result<Vec<_>, _>>()
        .map_err(|_| "video.conf needs width height fps integers")?;
    if values.len() != 3 { return Err("video.conf needs exactly three integers".into()); }
    let (width, height, fps) = (values[0], values[1], values[2]);
    if !(16..=1920).contains(&width) || !(16..=1080).contains(&height)
        || width % 2 != 0 || height % 2 != 0 || !(1..=60).contains(&fps) {
        return Err("video.conf dimensions/fps outside test source bounds".into());
    }
    Ok(VideoConfig { width, height, fps })
}

fn check_pcm(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() != 44100 * 4 * 2 || bytes.len() % AUDIO_CHUNK != 0 {
        return Err("tone.pcm must be exactly 2 seconds of 44100 Hz stereo PCM16LE".into());
    }
    Ok(())
}

// Split Annex-B without mistaking the third byte of a four-byte code for a
// second delimiter. Transmission is normalized to four-byte start codes.
fn nals(data: &[u8]) -> Result<Vec<&[u8]>, String> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let length = if data[i..].starts_with(&[0, 0, 0, 1]) { 4 }
            else if data[i..].starts_with(&[0, 0, 1]) { 3 } else { i += 1; continue; };
        if starts.len() >= 8192 { return Err("too many H264 NAL units".into()); }
        starts.push((i, i + length)); i += length;
    }
    if starts.first().map(|v| v.0) != Some(0) { return Err("H264 must begin with Annex-B start code".into()); }
    let mut result = Vec::new();
    for (index, &(_, start)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map(|v| v.0).unwrap_or(data.len());
        let nal = &data[start..end];
        if nal.is_empty() || nal[0] & 0x80 != 0 || !(1..=12).contains(&(nal[0] & 31)) {
            return Err("empty or invalid H264 NAL unit".into());
        }
        result.push(nal);
    }
    Ok(result)
}

fn append_nal(out: &mut Vec<u8>, nal: &[u8]) -> Result<(), String> {
    if nal.len() + 4 > MAX_FRAME.saturating_sub(out.len()) { return Err("H264 access unit exceeds 512 KiB".into()); }
    out.extend_from_slice(&[0, 0, 0, 1]); out.extend_from_slice(nal); Ok(())
}

struct Bits { data: Vec<u8>, bit: usize }
impl Bits {
    fn get(&mut self, count: usize) -> Result<u32, String> {
        if count > 32 || self.bit + count > self.data.len() * 8 { return Err("truncated H264 SPS".into()); }
        let mut value = 0;
        for _ in 0..count { value = (value << 1) | ((self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1) as u32; self.bit += 1; }
        Ok(value)
    }
    fn ue(&mut self) -> Result<u32, String> {
        let mut zeros = 0;
        while self.get(1)? == 0 {
            zeros += 1;
            if zeros > 24 { return Err("H264 SPS Exp-Golomb value exceeds bound".into()); }
        }
        Ok((1u32 << zeros) - 1 + self.get(zeros)?)
    }
}

// Baseline SPS only, matching our generator. Validate actual coded dimensions
// and VUI rate before advertising video.conf; no H264 decoding is performed.
fn check_sps(nal: &[u8], cfg: VideoConfig) -> Result<(), String> {
    let mut rbsp = Vec::new();
    let mut zeros = 0;
    for &byte in nal.get(1..).ok_or("missing SPS NAL header")? {
        if zeros >= 2 && byte == 3 { zeros = 0; continue; }
        rbsp.push(byte); zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    let mut bits = Bits { data: rbsp, bit: 0 };
    if bits.get(8)? != 66 { return Err("test video requires H264 baseline profile".into()); }
    bits.get(8)?; bits.get(8)?; bits.ue()?;
    if bits.ue()? > 12 { return Err("invalid SPS frame number size".into()); }
    match bits.ue()? {
        0 => { if bits.ue()? > 12 { return Err("invalid SPS picture order size".into()); } }
        2 => {},
        _ => return Err("unsupported SPS picture order type".into()),
    }
    bits.ue()?; bits.get(1)?;
    let width_mbs = bits.ue()? as u64 + 1;
    let height_mbs = bits.ue()? as u64 + 1;
    if bits.get(1)? != 1 { return Err("test video must be progressive".into()); }
    bits.get(1)?;
    let mut crop = [0u64; 4];
    if bits.get(1)? != 0 { for value in &mut crop { *value = bits.ue()? as u64; } }
    let width = (width_mbs * 16).checked_sub((crop[0] + crop[1]) * 2).ok_or("invalid SPS crop")?;
    let height = (height_mbs * 16).checked_sub((crop[2] + crop[3]) * 2).ok_or("invalid SPS crop")?;
    if (width, height) != (cfg.width as u64, cfg.height as u64) { return Err("SPS dimensions disagree with video.conf".into()); }
    if bits.get(1)? == 0 { return Err("test SPS needs VUI frame timing".into()); }
    if bits.get(1)? != 0 && bits.get(8)? == 255 { bits.get(16)?; bits.get(16)?; }
    if bits.get(1)? != 0 { bits.get(1)?; }
    if bits.get(1)? != 0 {
        bits.get(3)?; bits.get(1)?;
        if bits.get(1)? != 0 { bits.get(8)?; bits.get(8)?; bits.get(8)?; }
    }
    if bits.get(1)? != 0 { bits.ue()?; bits.ue()?; }
    if bits.get(1)? == 0 { return Err("SPS timing_info missing".into()); }
    let tick = bits.get(32)? as u64;
    let scale = bits.get(32)? as u64;
    bits.get(1)?;
    if tick == 0 || scale != tick * 2 * cfg.fps as u64 { return Err("SPS fps disagrees with video.conf".into()); }
    Ok(())
}

fn video(data: &[u8]) -> Result<(Vec<Frame>, Vec<u8>), String> {
    let mut frames = Vec::new();
    let mut current = Vec::new();
    let mut aud = false;
    let mut idr = false;
    let mut sets: [Option<Vec<u8>>; 2] = [None, None];
    for nal in nals(data)? {
        match nal[0] & 31 {
            9 => {
                if aud {
                    if current.is_empty() { return Err("AUD access unit has no picture".into()); }
                    if frames.is_empty() && (!idr || sets.iter().any(Option::is_none)) {
                        return Err("first video frame must contain SPS, PPS and IDR".into());
                    }
                    frames.push(Frame { data: std::mem::take(&mut current) });
                    if frames.len() >= 600 { return Err("too many test video frames".into()); }
                }
                aud = true; idr = false;
            }
            7 | 8 => {
                if !aud || !current.is_empty() { return Err("SPS/PPS must precede picture after AUD".into()); }
                let slot = &mut sets[(nal[0] & 31) as usize - 7];
                if let Some(old) = slot {
                    if old.as_slice() != nal { return Err("changing SPS/PPS unsupported in test loop".into()); }
                } else { *slot = Some(nal.to_vec()); }
            }
            1 | 5 => {
                if !aud { return Err("test H264 requires AUD access-unit boundaries".into()); }
                idr |= nal[0] & 31 == 5;
                append_nal(&mut current, nal)?;
            }
            // Strip AUD, SEI and end/filler markers: first transmitted frame
            // starts SPS+PPS+IDR; subsequent IDRs start directly with type 5.
            6 | 10 | 11 | 12 => {
                if !aud { return Err("H264 NAL appears before first AUD".into()); }
            }
            _ => return Err("unsupported H264 slice partition".into()),
        }
    }
    if current.is_empty() { return Err("test video has empty final access unit".into()); }
    if frames.is_empty() && (!idr || sets.iter().any(Option::is_none)) {
        return Err("first video frame must contain SPS, PPS and IDR".into());
    }
    frames.push(Frame { data: current });
    let mut prefix = Vec::new();
    for set in sets { append_nal(&mut prefix, &set.ok_or("missing SPS/PPS")?)?; }
    if prefix.len() + frames[0].data.len() > MAX_FRAME { return Err("initial H264 frame exceeds 512 KiB".into()); }
    Ok((frames, prefix))
}

impl TestMedia {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let cfg = config(&bounded_read(&dir.join("video.conf"), 128)?)?;
        let (frames, parameter_sets) = video(&bounded_read(&dir.join("video.h264"), MAX_VIDEO)?)?;
        check_sps(nals(&parameter_sets)?[0], cfg)?;
        // The generator always writes exactly two seconds; catch stale config
        // and incomplete assets before negotiating them with a head unit.
        if frames.len() != cfg.fps as usize * 2 { return Err("test video frame count must equal two seconds at configured fps".into()); }
        let pcm = bounded_read(&dir.join("tone.pcm"), MAX_AUDIO)?;
        check_pcm(&pcm)?;
        Ok(Self { config: cfg, fps: cfg.fps, frames, parameter_sets, pcm,
            video_playing: false, audio_playing: false, first_video: true,
            frame: 0, audio_offset: 0, video_due: Duration::ZERO,
            audio_due: Duration::ZERO, last_poll: Duration::ZERO })
    }

    pub fn config(&self) -> VideoConfig { self.config }

    pub fn set_fps(&mut self, fps: u32) -> Result<(), String> {
        if fps == 0 || fps > self.config.fps { return Err("test fps must be 1..source fps".into()); }
        self.fps = fps; Ok(())
    }

    /// Calling true again explicitly requests a fresh decodable keyframe.
    pub fn set_playing(&mut self, playing: bool, now: Duration) {
        self.video_playing = playing;
        if playing { self.frame = 0; self.first_video = true; self.video_due = now; }
    }

    pub fn set_audio_playing(&mut self, playing: bool, now: Duration) {
        self.audio_playing = playing;
        if playing { self.audio_offset = 0; self.audio_due = now; }
    }

    pub fn audio_init(&self) -> Message {
        // Protobuf int32: sampleRate=44100, channels=2, sampleFormat=16.
        media_message(Channel::Media, 0x30001, 0, vec![8, 0xc4, 0xd8, 2, 16, 2, 24, 16])
    }

    pub fn audio_stop(&self) -> Message { media_message(Channel::Media, 0x30002, 0, Vec::new()) }

    /// At most one due packet per call. Late polling schedules forward from
    /// now instead of accumulating a burst of overdue media allocations.
    pub fn next(&mut self, now: Duration) -> Result<Option<Message>, String> {
        if now < self.last_poll { return Err("test media clock moved backwards".into()); }
        self.last_poll = now;
        let video = self.video_playing && now >= self.video_due;
        let audio = self.audio_playing && now >= self.audio_due;
        if !video && !audio { return Ok(None); }
        let time = u32::try_from(now.as_millis()).map_err(|_| "test media timestamp exceeds u32 milliseconds")?;
        if video && (!audio || self.video_due <= self.audio_due) {
            let frame = &self.frames[self.frame];
            let mut data = Vec::with_capacity(frame.data.len() + if self.first_video { self.parameter_sets.len() } else { 0 });
            if self.first_video { data.extend_from_slice(&self.parameter_sets); self.first_video = false; }
            data.extend_from_slice(&frame.data);
            self.frame = (self.frame + 1) % self.frames.len();
            let interval = Duration::from_nanos(1_000_000_000 / self.fps as u64);
            self.video_due += interval;
            if self.video_due <= now { self.video_due = now + interval; }
            Ok(Some(media_message(Channel::Video, 0x20001, time, data)))
        } else {
            let data = self.pcm[self.audio_offset..self.audio_offset + AUDIO_CHUNK].to_vec();
            self.audio_offset = (self.audio_offset + AUDIO_CHUNK) % self.pcm.len();
            self.audio_due += Duration::from_millis(20);
            if self.audio_due <= now { self.audio_due = now + Duration::from_millis(20); }
            Ok(Some(media_message(Channel::Media, 0x30006, time, data)))
        }
    }
}

fn media_message(channel: Channel, id: u32, timestamp: u32, payload: Vec<u8>) -> Message {
    Message { channel, id, timestamp_ms: Some(timestamp), reserved: 0, payload }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<u8> {
        vec![0,0,0,1,9,0xf0, 0,0,1,0x67,0x42, 0,0,0,1,0x68,0xce,
            0,0,1,0x65,0x88, 0,0,0,1,9,0xf0, 0,0,1,0x65,0x99]
    }
    fn source() -> TestMedia {
        let (frames, parameter_sets) = video(&sample()).unwrap();
        TestMedia { config: VideoConfig { width: 800, height: 480, fps: 30 }, fps: 30,
            frames, parameter_sets, pcm: vec![0; AUDIO_CHUNK * 2], video_playing: false,
            audio_playing: false, first_video: true, frame: 0, audio_offset: 0,
            video_due: Duration::ZERO, audio_due: Duration::ZERO, last_poll: Duration::ZERO }
    }
    #[test]
    fn config_is_strict_and_bounded() {
        assert_eq!(config(b"800 480 30\n").unwrap().fps, 30);
        for bad in ["800 480", "800 480 30 1", "801 480 30", "800 480 0", "999999 480 30", "-1 480 30"] {
            assert!(config(bad.as_bytes()).is_err(), "{bad}");
        }
    }
    #[test]
    fn pcm_rejects_empty_truncated_unaligned_and_oversized_assets() {
        let valid = vec![0; 44100 * 4 * 2];
        check_pcm(&valid).unwrap();
        for count in [0, 4, valid.len() - 1, valid.len() - 4] { assert!(check_pcm(&valid[..count]).is_err()); }
        assert!(check_pcm(&vec![0; MAX_AUDIO + 1]).is_err());
    }
    #[test]
    fn actual_baseline_sps_must_match_configured_dimensions_and_rate() {
        // Synthetic ffmpeg/x264 output for 800x480@30, including EPB bytes.
        let sps = [0x67,0x42,0xc0,0x1f,0xda,0x03,0x20,0xf6,0xc0,0x44,0,0,3,0,4,0,0,3,0,0xf2,0x3c,0x60,0xca,0x80];
        let cfg = VideoConfig { width: 800, height: 480, fps: 30 };
        check_sps(&sps, cfg).unwrap();
        assert!(check_sps(&sps, VideoConfig { width: 1280, ..cfg }).is_err());
        assert!(check_sps(&sps, VideoConfig { fps: 25, ..cfg }).is_err());
        for end in 0..16 { assert!(check_sps(&sps[..end], cfg).is_err()); }
    }
    #[test]
    fn aud_is_removed_and_startup_is_sps_pps_idr() {
        let (frames, prefix) = video(&sample()).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(prefix[4] & 31, 7);
        assert_eq!(frames[0].data[4] & 31, 5);
        assert_eq!(frames[1].data[4] & 31, 5);
        for bad in [vec![], vec![0,0,1], vec![0,0,1,0x80], vec![0,0,1,0x65,1],
            vec![0,0,1,9,0xf0,0,0,1,0x41,1]] { assert!(video(&bad).is_err()); }
        let mut oversized = sample(); oversized.extend(vec![0x55; MAX_FRAME]);
        assert!(video(&oversized).is_err());
    }
    #[test]
    fn opt_in_independent_clocks_keyframe_resume_and_no_backlog() {
        let mut s = source();
        assert!(s.next(Duration::ZERO).unwrap().is_none());
        s.set_playing(true, Duration::ZERO);
        assert_eq!(s.next(Duration::ZERO).unwrap().unwrap().payload[4] & 31, 7);
        assert!(s.next(Duration::ZERO).unwrap().is_none());
        assert_eq!(s.next(Duration::from_secs(10)).unwrap().unwrap().payload[4] & 31, 5);
        assert!(s.next(Duration::from_secs(10)).unwrap().is_none());
        s.set_playing(false, Duration::from_secs(10));
        s.set_audio_playing(true, Duration::from_secs(10));
        let audio = s.next(Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!((audio.id, audio.payload.len()), (0x30006, AUDIO_CHUNK));
        s.set_playing(true, Duration::from_secs(11));
        assert_eq!(s.next(Duration::from_secs(11)).unwrap().unwrap().id, 0x30006);
        let resumed = s.next(Duration::from_secs(11)).unwrap().unwrap();
        assert_eq!(resumed.payload[4] & 31, 7);
        assert_eq!(resumed.timestamp_ms, Some(11000));
        assert!(s.next(Duration::ZERO).is_err());
        assert!(s.set_fps(0).is_err()); assert!(s.set_fps(31).is_err());
        s.set_fps(3).unwrap();
        assert_eq!(s.audio_init().payload, vec![8,0xc4,0xd8,2,16,2,24,16]);
    }
}
