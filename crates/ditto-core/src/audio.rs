use crate::{
    check_cancel,
    process::{run, ProcessOptions, ProcessOutput},
    Cancellation, Error, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Per-export decoded PCM budget; staging needs up to roughly twice this space.
pub const MAX_PCM_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Per-preview WAV budget, including container overhead.
pub const MAX_PREVIEW_BYTES: u64 = 1024 * 1024 * 1024;
const FORMATS: &str = "wav,mp3,mov,mp4,m4a,3gp,3g2,mj2,flac,ogg,matroska,webm,aac,aiff";
#[derive(Clone, Debug)]
pub struct Engine {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioInfo {
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u16,
}
fn local_input(path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.is_file() {
        return Err(Error::Invalid("input must be a regular file".into()));
    }
    Ok(path)
}
fn success(output: ProcessOutput) -> Result<ProcessOutput> {
    if !output.status.success() {
        return Err(Error::Helper(
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect(),
        ));
    }
    Ok(output)
}
/// Conservative application filename policy, not a documented pedal naming rule.
pub fn safe_stem(title: &str) -> String {
    let title = if title.to_ascii_lowercase().ends_with(".wav") {
        &title[..title.len() - 4]
    } else {
        title
    };
    let stem: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " _-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let stem = stem.trim_matches([' ', '_', '-']);
    if stem.is_empty() {
        return "Track".into();
    }
    let upper = stem.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "B", "BT", "LOOP"].contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'));
    if reserved {
        format!("Track_{stem}")
    } else {
        stem.to_owned()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WavInfo {
    pub samples: u64,
    pub peak: u32,
    pub duration: f64,
}

/// Validate our deliberately narrow classic PCM profile and decode every sample.
pub fn validate_wav(path: &Path, cancel: &Cancellation) -> Result<WavInfo> {
    use std::io::Read;
    check_cancel(cancel)?;
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mut header = [0u8; 44];
    file.read_exact(&mut header)?;
    let u16_at = |n| u16::from_le_bytes(header[n..n + 2].try_into().unwrap());
    let u32_at = |n| u32::from_le_bytes(header[n..n + 4].try_into().unwrap());
    let data_size = u64::from(u32_at(40));
    if &header[..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || u32_at(16) != 16
        || u16_at(20) != 1
        || u16_at(22) != 1
        || u32_at(24) != 44100
        || u32_at(28) != 132300
        || u16_at(32) != 3
        || u16_at(34) != 24
        || &header[36..40] != b"data"
        || data_size == 0
        || data_size % 3 != 0
        || length != 44 + data_size + data_size % 2
        || u64::from(u32_at(4)) + 8 != length
    {
        return Err(Error::Invalid(
            "invalid or truncated classic 24-bit mono 44100 Hz WAV".into(),
        ));
    }
    let mut remaining = data_size;
    let mut peak = 0;
    let mut buffer = [0; 3 * 16384];
    while remaining > 0 {
        check_cancel(cancel)?;
        let n = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..n])?;
        for sample in buffer[..n].as_chunks::<3>().0 {
            let decoded = i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8;
            peak = peak.max(decoded.unsigned_abs());
        }
        remaining -= n as u64;
    }
    if data_size % 2 != 0 {
        let mut pad = [0];
        file.read_exact(&mut pad)?;
        if pad[0] != 0 {
            return Err(Error::Invalid("nonzero RIFF padding".into()));
        }
    }
    let samples = data_size / 3;
    Ok(WavInfo {
        samples,
        peak,
        duration: samples as f64 / 44100.0,
    })
}

impl Engine {
    /// Caller owns the returned preview and must remove it after playback ends.
    pub fn preview(&self, input: &Path, work_dir: &Path, cancel: &Cancellation) -> Result<PathBuf> {
        let info = self.probe(input, cancel)?;
        if info.duration * f64::from(info.sample_rate) * f64::from(info.channels.min(2)) * 2.0
            + 4096.0
            > MAX_PREVIEW_BYTES as f64
        {
            return Err(Error::Invalid("decoded preview exceeds size limit".into()));
        }
        let input = local_input(input)?;
        let staged = tempfile::Builder::new()
            .prefix("ditto-preview-")
            .suffix(".wav")
            .tempfile_in(work_dir)?;
        let mut command = Command::new(&self.ffmpeg);
        command
            .env_remove("FFREPORT")
            .env("AV_LOG_FORCE_NOCOLOR", "1");
        command
            .args([
                "-nostdin",
                "-hide_banner",
                "-v",
                "error",
                "-xerror",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                FORMATS,
                "-i",
            ])
            .arg(input)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-af",
                "asetpts=PTS-STARTPTS",
                "-ac",
                &info.channels.min(2).to_string(),
                "-ar",
                &info.sample_rate.to_string(),
                "-c:a",
                "pcm_s16le",
                "-map_metadata",
                "-1",
                "-f",
                "wav",
                "-fs",
                &(MAX_PREVIEW_BYTES + 1).to_string(),
                "-y",
            ])
            .arg(staged.path());
        success(run(
            &mut command,
            cancel,
            ProcessOptions::default(),
            |_| {},
        )?)?;
        check_cancel(cancel)?;
        if staged.as_file().metadata()?.len() > MAX_PREVIEW_BYTES {
            return Err(Error::Invalid("decoded preview exceeds size limit".into()));
        }
        let preview_info = self.probe(staged.path(), cancel)?;
        if preview_info.sample_rate != info.sample_rate
            || preview_info.channels != info.channels.min(2)
        {
            return Err(Error::Invalid("invalid decoded preview".into()));
        }
        staged.as_file().sync_all()?;
        let (_, path) = staged.keep().map_err(|error| Error::Io(error.error))?;
        Ok(path)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn convert(
        &self,
        input: &Path,
        output_dir: &Path,
        title: &str,
        trim_start: f64,
        trim_end: Option<f64>,
        cancel: &Cancellation,
        on_progress: impl Fn(f64),
    ) -> Result<PathBuf> {
        self.convert_inner(
            input,
            output_dir,
            title,
            trim_start,
            trim_end,
            cancel,
            on_progress,
            None,
        )
    }

    /// Like `convert`, with one create-new, app-owned recovery journal per active job.
    #[allow(clippy::too_many_arguments)]
    pub fn convert_tracked(
        &self,
        input: &Path,
        output_dir: &Path,
        title: &str,
        trim_start: f64,
        trim_end: Option<f64>,
        cancel: &Cancellation,
        on_progress: impl Fn(f64),
        journal_path: &Path,
    ) -> Result<PathBuf> {
        self.convert_inner(
            input,
            output_dir,
            title,
            trim_start,
            trim_end,
            cancel,
            on_progress,
            Some(journal_path),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn convert_inner(
        &self,
        input: &Path,
        output_dir: &Path,
        title: &str,
        trim_start: f64,
        trim_end: Option<f64>,
        cancel: &Cancellation,
        on_progress: impl Fn(f64),
        journal_path: Option<&Path>,
    ) -> Result<PathBuf> {
        let info = self.probe(input, cancel)?;
        if !trim_start.is_finite()
            || trim_start < 0.0
            || trim_start >= info.duration
            || trim_end
                .is_some_and(|end| !end.is_finite() || end <= trim_start || end > info.duration)
        {
            return Err(Error::Invalid(
                "trim must be a finite, nonempty range inside the audio duration".into(),
            ));
        }
        if (trim_end.unwrap_or(info.duration) - trim_start) * 132300.0 > MAX_PCM_BYTES as f64 {
            return Err(Error::Invalid("decoded audio exceeds size limit".into()));
        }
        let start_sample = (trim_start * f64::from(info.sample_rate)).round();
        let end_sample = (trim_end.unwrap_or(info.duration) * f64::from(info.sample_rate)).round();
        if end_sample <= start_sample
            || start_sample > u64::MAX as f64
            || end_sample > u64::MAX as f64
        {
            return Err(Error::Invalid(
                "trim must retain at least one input sample".into(),
            ));
        }
        let input = local_input(input)?;
        // Declaration order makes staging cleanup precede journal cleanup.
        let _journal;
        let stage = tempfile::Builder::new()
            .prefix(".ditto-")
            .tempdir_in(output_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(stage.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let mut staged = tempfile::Builder::new()
            .prefix(".ditto-wav-")
            .rand_bytes(16)
            .tempfile_in(stage.path())?;
        _journal = journal_path
            .map(|path| crate::staging::Journal::create(path, stage.path(), staged.path()))
            .transpose()?;
        let raw_path = stage.path().join("audio.pcm");
        let mut raw_options = std::fs::OpenOptions::new();
        raw_options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            raw_options.mode(0o600);
        }
        drop(raw_options.open(&raw_path)?);
        let start = (trim_start * f64::from(info.sample_rate)).round() as u64;
        let end = trim_end.map(|end| (end * f64::from(info.sample_rate)).round() as u64);
        let mut filter = format!("asetpts=PTS-STARTPTS,atrim=start_sample={start}");
        if let Some(end) = end {
            filter.push_str(&format!(":end_sample={end}"));
        }
        filter.push_str(",asetpts=PTS-STARTPTS");
        let mut command = Command::new(&self.ffmpeg);
        command
            .env_remove("FFREPORT")
            .env("AV_LOG_FORCE_NOCOLOR", "1");
        command
            .args([
                "-nostdin",
                "-hide_banner",
                "-v",
                "error",
                "-xerror",
                "-nostats",
                "-progress",
                "pipe:1",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                FORMATS,
                "-i",
            ])
            .arg(input)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-af",
                &filter,
                "-ac",
                "1",
                "-ar",
                "44100",
                "-c:a",
                "pcm_s24le",
                "-f",
                "s24le",
                "-fs",
                &(MAX_PCM_BYTES + 1).to_string(),
                "-y",
            ])
            .arg(&raw_path);
        let retained_duration = trim_end.unwrap_or(info.duration) - trim_start;
        let mut progress_bytes = Vec::new();
        let mut last_progress = 0.0;
        success(run(
            &mut command,
            cancel,
            ProcessOptions {
                timeout: Duration::from_secs(3600),
                max_output: 8 * 1024 * 1024,
            },
            |bytes| {
                progress_bytes.extend_from_slice(bytes);
                while let Some(end) = progress_bytes.iter().position(|&b| b == b'\n') {
                    let line = String::from_utf8_lossy(&progress_bytes[..end]);
                    if let Some(microseconds) = line
                        .strip_prefix("out_time_us=")
                        .and_then(|s| s.trim().parse::<f64>().ok())
                        .filter(|n| n.is_finite() && *n >= 0.0)
                    {
                        let fraction =
                            (microseconds / 1_000_000.0 / retained_duration).clamp(0.0, 0.99);
                        if fraction > last_progress {
                            last_progress = fraction;
                            on_progress(fraction);
                        }
                    }
                    progress_bytes.drain(..=end);
                }
            },
        )?)?;
        check_cancel(cancel)?;
        let mut raw = std::fs::File::open(raw_path)?;
        if raw.metadata()?.len() > MAX_PCM_BYTES {
            return Err(Error::Invalid("decoded audio exceeds size limit".into()));
        }
        let data_size = u32::try_from(raw.metadata()?.len())
            .map_err(|_| Error::Invalid("audio exceeds classic WAV size limit".into()))?;
        if data_size == 0 || data_size % 3 != 0 {
            return Err(Error::Invalid("empty or incomplete PCM output".into()));
        }
        use std::io::Write;
        let file = staged.as_file_mut();
        file.write_all(b"RIFF")?;
        file.write_all(&(36 + data_size + data_size % 2).to_le_bytes())?;
        file.write_all(b"WAVEfmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&44100u32.to_le_bytes())?;
        file.write_all(&132300u32.to_le_bytes())?;
        file.write_all(&3u16.to_le_bytes())?;
        file.write_all(&24u16.to_le_bytes())?;
        file.write_all(b"data")?;
        file.write_all(&data_size.to_le_bytes())?;
        use std::io::Read;
        let mut remaining = data_size as usize;
        let mut copy_buffer = [0; 48 * 1024];
        while remaining > 0 {
            check_cancel(cancel)?;
            let count = remaining.min(copy_buffer.len());
            raw.read_exact(&mut copy_buffer[..count])?;
            file.write_all(&copy_buffer[..count])?;
            remaining -= count;
        }
        if data_size % 2 != 0 {
            file.write_all(&[0])?;
        }
        file.sync_all()?;
        let validated = validate_wav(staged.path(), cancel)?;
        if let Some(end) = end {
            let expected =
                ((end - start) as f64 * 44100.0 / f64::from(info.sample_rate)).round() as u64;
            if validated.samples.abs_diff(expected) > 1 {
                return Err(Error::Invalid(
                    "decoded audio did not fill the requested sample range".into(),
                ));
            }
        }
        let title = safe_stem(title);
        for number in 1..=10000 {
            check_cancel(cancel)?;
            let name = if number == 1 {
                format!("{title}.wav")
            } else {
                format!("{title} ({number}).wav")
            };
            let output = output_dir.join(name);
            match staged.persist_noclobber(&output) {
                Ok(_) => {
                    on_progress(1.0);
                    return Ok(output);
                }
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    staged = error.file;
                }
                Err(error) => return Err(Error::Io(error.error)),
            }
        }
        Err(Error::Invalid("too many filename collisions".into()))
    }

    pub fn probe(&self, input: &Path, cancel: &Cancellation) -> Result<AudioInfo> {
        check_cancel(cancel)?;
        let input = local_input(input)?;
        let mut command = Command::new(&self.ffprobe);
        command
            .env_remove("FFREPORT")
            .env("AV_LOG_FORCE_NOCOLOR", "1");
        command
            .args([
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                FORMATS,
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=sample_rate,channels,duration:format=duration",
                "-of",
                "json",
            ])
            .arg(input);
        let output = success(run(
            &mut command,
            cancel,
            ProcessOptions {
                timeout: Duration::from_secs(30),
                max_output: 1024 * 1024,
            },
            |_| {},
        )?)?;
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let stream = value["streams"]
            .as_array()
            .and_then(|streams| streams.first())
            .ok_or_else(|| Error::Invalid("no audio stream found".into()))?;
        let sample_rate = stream["sample_rate"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&rate| rate > 0 && rate <= 768000)
            .ok_or_else(|| Error::Invalid("invalid sample rate".into()))?;
        let channels = stream["channels"]
            .as_u64()
            .filter(|&n| n > 0 && n <= 64)
            .ok_or_else(|| Error::Invalid("invalid audio channel count".into()))?
            as u16;
        let duration = [
            stream["duration"].as_str(),
            value["format"]["duration"].as_str(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|s| s.parse::<f64>().ok())
        .find(|n| n.is_finite() && *n > 0.0)
        .ok_or_else(|| Error::Invalid("audio duration unavailable or invalid".into()))?;
        Ok(AudioInfo {
            duration,
            sample_rate,
            channels,
        })
    }
}
