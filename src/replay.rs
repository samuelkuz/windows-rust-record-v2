use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{AppResult, config::RecorderConfig, ffmpeg};

const STABLE_COPY_ATTEMPTS: usize = 3;
const STABLE_COPY_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(crate) struct ReplayBuffer {
    config: RecorderConfig,
    segment_dir: PathBuf,
    clip_dir: PathBuf,
}

impl ReplayBuffer {
    pub(crate) fn new(config: RecorderConfig) -> AppResult<Self> {
        let segment_dir = config.segment_dir();
        fs::create_dir_all(&segment_dir)?;
        clear_segment_dir(&segment_dir)?;

        let clip_dir = config.clip_dir();
        fs::create_dir_all(&clip_dir)?;

        Ok(Self {
            config,
            segment_dir,
            clip_dir,
        })
    }

    pub(crate) fn segment_path_pattern(&self) -> PathBuf {
        self.segment_dir.join("segment-%03d.ts")
    }

    pub(crate) fn save_recent_clip(&self, seconds: u64) -> AppResult<PathBuf> {
        let segments = self.recent_segments(seconds)?;
        if segments.is_empty() {
            tracing::warn!(seconds, "no replay segments are available yet");
            return Err("No replay segments are available yet".into());
        }

        let export_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let export_dir = self.clip_dir.join(format!("clip-work-{export_id}"));
        fs::create_dir_all(&export_dir)?;
        tracing::info!(
            seconds,
            export_dir = %export_dir.display(),
            segment_count = segments.len(),
            "exporting recent replay clip"
        );

        let copied_segments = copy_stable_segments(&segments, &export_dir)?;
        if copied_segments.is_empty() {
            tracing::warn!("no replay segments could be copied for export");
            return Err("No replay segments could be copied for export".into());
        }

        let concat_list =
            write_concat_list(&copied_segments, &export_dir, self.config.segment_seconds)?;
        let clip_path = self.clip_dir.join(format!("clip-{export_id}.mp4"));
        let temp_clip_path = self.clip_dir.join(format!("clip-{export_id}.tmp.mp4"));
        save_validated_clip(&concat_list, &temp_clip_path, &clip_path)?;

        if let Err(error) = fs::remove_dir_all(&export_dir) {
            tracing::warn!(
                path = %export_dir.display(),
                error = %error,
                "could not remove temporary replay export directory"
            );
            eprintln!(
                "Could not remove temporary replay export directory {}: {error}",
                export_dir.display()
            );
        }

        Ok(clip_path)
    }

    fn recent_segments(&self, seconds: u64) -> AppResult<Vec<PathBuf>> {
        let now = SystemTime::now();
        let cutoff = now
            .checked_sub(Duration::from_secs(seconds + self.config.segment_seconds))
            .ok_or("System clock underflow while selecting replay segments")?;

        let mut segments = fs::read_dir(&self.segment_dir)?
            .filter_map(Result::ok)
            .filter(|entry| has_extension(&entry.path(), "ts"))
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                (modified >= cutoff).then_some((modified, entry.path()))
            })
            .collect::<Vec<_>>();

        segments.sort_by_key(|(modified, _)| *modified);
        segments.pop();
        Ok(segments.into_iter().map(|(_, path)| path).collect())
    }
}

fn copy_stable_segments(segments: &[PathBuf], export_dir: &Path) -> AppResult<Vec<PathBuf>> {
    let mut copied_segments = Vec::new();
    for (index, source) in segments.iter().enumerate() {
        let destination = export_dir.join(format!("segment-{index:03}.ts"));
        match copy_stable_segment(source, &destination) {
            Ok(()) => copied_segments.push(destination),
            Err(error) => {
                tracing::warn!(
                    source = %source.display(),
                    error = %error,
                    "skipping replay segment because it was not stable during export"
                );
                eprintln!(
                    "Skipping replay segment {} because it was not stable during export: {error}",
                    source.display()
                );
            }
        }
    }

    Ok(copied_segments)
}

fn copy_stable_segment(source: &Path, destination: &Path) -> AppResult<()> {
    for attempt in 1..=STABLE_COPY_ATTEMPTS {
        let before = fs::metadata(source)?;
        let copied_bytes = fs::copy(source, destination)?;
        let after = fs::metadata(source)?;

        if before.len() == after.len()
            && before.modified().ok() == after.modified().ok()
            && copied_bytes == after.len()
        {
            return Ok(());
        }

        if let Err(error) = fs::remove_file(destination) {
            tracing::warn!(
                path = %destination.display(),
                error = %error,
                "could not remove unstable copied segment"
            );
            eprintln!(
                "Could not remove unstable copied segment {}: {error}",
                destination.display()
            );
        }

        if attempt < STABLE_COPY_ATTEMPTS {
            thread::sleep(STABLE_COPY_RETRY_DELAY);
        }
    }

    Err("segment changed while it was being copied".into())
}

fn write_concat_list(
    copied_segments: &[PathBuf],
    export_dir: &Path,
    segment_seconds: u64,
) -> AppResult<PathBuf> {
    let concat_list = export_dir.join("segments.txt");
    let mut concat_file = fs::File::create(&concat_list)?;
    for segment in copied_segments {
        writeln!(concat_file, "file '{}'", ffmpeg::concat_path(segment))?;
        writeln!(concat_file, "duration {segment_seconds}.0")?;
    }
    Ok(concat_list)
}

fn save_validated_clip(
    concat_list: &Path,
    temp_clip_path: &Path,
    clip_path: &Path,
) -> AppResult<()> {
    if let Err(error) = save_clip(concat_list, temp_clip_path)
        .and_then(|()| validate_clip(temp_clip_path))
        .and_then(|()| fs::rename(temp_clip_path, clip_path).map_err(Into::into))
    {
        if let Err(remove_error) = fs::remove_file(temp_clip_path) {
            if remove_error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %temp_clip_path.display(),
                    error = %remove_error,
                    "could not remove failed temporary clip"
                );
                eprintln!(
                    "Could not remove failed temporary clip {}: {remove_error}",
                    temp_clip_path.display()
                );
            }
        }

        return Err(error);
    }

    Ok(())
}

fn save_clip(concat_list: &Path, clip_path: &Path) -> AppResult<()> {
    let tools = ffmpeg::FfmpegTools::resolve()?;
    let output = Command::new(tools.ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-fflags",
            "+genpts",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
        ])
        .arg(concat_list)
        .args(["-c", "copy", "-movflags", "+faststart", "-y"])
        .arg(clip_path)
        .output()
        .map_err(|error| {
            format!(
                "Could not start ffmpeg at {} to save replay clip: {error}",
                tools.ffmpeg().display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg failed to save replay clip: {stderr}").into());
    }

    Ok(())
}

fn validate_clip(clip_path: &Path) -> AppResult<()> {
    let tools = ffmpeg::FfmpegTools::resolve()?;
    let output = Command::new(tools.ffprobe())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(clip_path)
        .output()
        .map_err(|error| {
            format!(
                "Could not start ffprobe at {} to validate replay clip: {error}",
                tools.ffprobe().display()
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe failed to validate replay clip: {stderr}").into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.lines().any(|line| line.trim() == "video") {
        return Err("ffprobe did not find a video stream in the exported replay clip".into());
    }

    Ok(())
}

fn clear_segment_dir(segment_dir: &Path) -> AppResult<()> {
    for entry in fs::read_dir(segment_dir)? {
        let entry = entry?;
        let path = entry.path();
        if has_extension(&path, "ts") {
            fs::remove_file(path)?;
        }
    }

    Ok(())
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|path_extension| path_extension == extension)
}
