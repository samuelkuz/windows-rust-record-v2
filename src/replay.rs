use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{AppResult, config::SEGMENT_SECONDS, ffmpeg};

pub(crate) struct ReplayBuffer {
    segment_dir: PathBuf,
    clip_dir: PathBuf,
}

impl ReplayBuffer {
    pub(crate) fn new() -> AppResult<Self> {
        let mut segment_dir = std::env::current_dir()?;
        segment_dir.push("replay-segments");
        fs::create_dir_all(&segment_dir)?;
        clear_segment_dir(&segment_dir)?;

        let mut clip_dir = std::env::current_dir()?;
        clip_dir.push("clips");
        fs::create_dir_all(&clip_dir)?;

        Ok(Self {
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
            return Err("No replay segments are available yet".into());
        }

        let export_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let export_dir = self.clip_dir.join(format!("clip-work-{export_id}"));
        fs::create_dir_all(&export_dir)?;

        let copied_segments = copy_segments(&segments, &export_dir)?;
        if copied_segments.is_empty() {
            return Err("No replay segments could be copied for export".into());
        }

        let concat_list = write_concat_list(&copied_segments, &export_dir)?;
        let clip_path = self.clip_dir.join(format!("clip-{export_id}.mp4"));
        save_clip(&concat_list, &clip_path)?;

        if let Err(error) = fs::remove_dir_all(&export_dir) {
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
            .checked_sub(Duration::from_secs(seconds + SEGMENT_SECONDS))
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
        Ok(segments.into_iter().map(|(_, path)| path).collect())
    }
}

fn copy_segments(segments: &[PathBuf], export_dir: &Path) -> AppResult<Vec<PathBuf>> {
    let mut copied_segments = Vec::new();
    for (index, source) in segments.iter().enumerate() {
        let destination = export_dir.join(format!("segment-{index:03}.ts"));
        match fs::copy(source, &destination) {
            Ok(_) => copied_segments.push(destination),
            Err(error) => eprintln!(
                "Skipping replay segment {} because it could not be copied: {error}",
                source.display()
            ),
        }
    }

    Ok(copied_segments)
}

fn write_concat_list(copied_segments: &[PathBuf], export_dir: &Path) -> AppResult<PathBuf> {
    let concat_list = export_dir.join("segments.txt");
    let mut concat_file = fs::File::create(&concat_list)?;
    for segment in copied_segments {
        writeln!(concat_file, "file '{}'", ffmpeg::concat_path(segment))?;
    }
    Ok(concat_list)
}

fn save_clip(concat_list: &Path, clip_path: &Path) -> AppResult<()> {
    let output = Command::new("ffmpeg")
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
        .map_err(|error| format!("Could not start ffmpeg to save replay clip: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg failed to save replay clip: {stderr}").into());
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
