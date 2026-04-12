use std::{
    io::Write,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    AppResult, capture::PrimaryDisplayCapture, config::FRAME_RATE, ffmpeg, replay::ReplayBuffer,
};

pub(crate) fn start_capture_thread(
    capturer: PrimaryDisplayCapture,
    replay_buffer: Arc<ReplayBuffer>,
) -> AppResult<thread::JoinHandle<()>> {
    let mut encoder = ffmpeg::start_replay_encoder(
        capturer.width(),
        capturer.height(),
        replay_buffer.segment_path_pattern(),
    )?;
    let mut encoder_stdin = encoder
        .stdin
        .take()
        .ok_or("Could not open ffmpeg stdin for replay recording")?;

    Ok(thread::spawn(move || {
        let frame_interval = Duration::from_millis(1_000 / FRAME_RATE as u64);
        let mut last_frame: Option<Vec<u8>> = None;

        loop {
            let frame_started = Instant::now();
            match capturer.capture() {
                Ok(frame) => {
                    last_frame = Some(frame.pixels);
                    if let Some(frame_pixels) = &last_frame {
                        if let Err(error) = encoder_stdin.write_all(frame_pixels) {
                            eprintln!("Replay encoder stopped accepting frames: {error}");
                            break;
                        }
                    }
                }
                Err(error) => {
                    if let Some(frame_pixels) = &last_frame {
                        if let Err(write_error) = encoder_stdin.write_all(frame_pixels) {
                            eprintln!(
                                "Replay encoder stopped accepting repeated frames: {write_error}"
                            );
                            break;
                        }
                    } else {
                        eprintln!("Could not capture initial replay frame: {error}");
                    }
                }
            }

            let elapsed = frame_started.elapsed();
            if elapsed < frame_interval {
                thread::sleep(frame_interval - elapsed);
            }
        }

        drop(encoder_stdin);
        let _ = encoder.kill();
        let _ = encoder.wait();
    }))
}
