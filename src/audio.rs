use std::{
    error::Error,
    fs::File,
    io::Write,
    os::windows::io::{FromRawHandle, OwnedHandle},
    ptr, slice,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
        },
        Media::{
            Audio::{
                AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
                IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator,
                MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole,
                eRender,
            },
            KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE},
            Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT},
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_OUTBOUND},
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
            },
        },
    },
    core::{HRESULT, IUnknown, PCWSTR},
};

use crate::{AppResult, ffmpeg};

const AUDIO_PIPE_BUFFER_BYTES: u32 = 192_000;
const AUDIO_BUFFER_100NS: i64 = 10_000_000;
const SILENCE_POLL_MS: u64 = 5;
const MAX_SILENCE_WRITE_MS: u32 = 100;

pub(crate) struct PreparedLoopbackAudio {
    pipe_server: NamedPipeServer,
    pipe_path: String,
    spec: AudioSpec,
}

impl PreparedLoopbackAudio {
    pub(crate) fn ffmpeg_input(&self) -> ffmpeg::AudioInput {
        ffmpeg::AudioInput {
            pipe_path: self.pipe_path.clone(),
            sample_format: self.spec.ffmpeg_sample_format,
            sample_rate: self.spec.sample_rate,
            channels: self.spec.channels,
        }
    }

    pub(crate) fn start(self, stop_requested: Arc<AtomicBool>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Err(error) = run_loopback_capture(self.pipe_server, stop_requested) {
                if !is_expected_pipe_close(error.as_ref()) {
                    tracing::warn!(error = %error, "WASAPI loopback audio capture stopped");
                    eprintln!("WASAPI loopback audio capture stopped: {error}");
                }
            }
        })
    }
}

pub(crate) fn prepare_loopback_audio() -> AppResult<PreparedLoopbackAudio> {
    let pipe_path = unique_pipe_path()?;
    let pipe_server = NamedPipeServer::new(&pipe_path)?;
    let spec = probe_loopback_audio_spec()?;

    Ok(PreparedLoopbackAudio {
        pipe_server,
        pipe_path,
        spec,
    })
}

fn probe_loopback_audio_spec() -> AppResult<AudioSpec> {
    let _com = ComApartment::initialize()?;
    let audio_client = default_render_audio_client()?;
    let mix_format = MixFormat::get(&audio_client)?;
    unsafe { AudioSpec::from_wave_format(mix_format.as_ptr()) }
}

fn run_loopback_capture(
    pipe_server: NamedPipeServer,
    stop_requested: Arc<AtomicBool>,
) -> AppResult<()> {
    let mut pipe = pipe_server.connect()?;
    let _com = ComApartment::initialize()?;
    let capture = LoopbackCapture::open()?;
    tracing::info!(
        sample_rate = capture.spec.sample_rate,
        channels = capture.spec.channels,
        frame_bytes = capture.spec.frame_bytes,
        "WASAPI loopback audio capture started"
    );
    let _running = RunningAudioClient::start(&capture.audio_client)?;
    let mut timeline = AudioTimeline::new(capture.spec, capture.silence_fill_delay_frames);

    while !stop_requested.load(Ordering::Relaxed) {
        let mut packet_frames = unsafe { capture.capture_client.GetNextPacketSize()? };
        if packet_frames == 0 {
            timeline.write_due_silence(&mut pipe)?;
            thread::sleep(Duration::from_millis(SILENCE_POLL_MS));
            continue;
        }

        while packet_frames > 0 {
            if stop_requested.load(Ordering::Relaxed) {
                break;
            }

            let mut data = ptr::null_mut::<u8>();
            let mut frame_count = 0_u32;
            let mut flags = 0_u32;

            unsafe {
                capture.capture_client.GetBuffer(
                    &mut data,
                    &mut frame_count,
                    &mut flags,
                    None,
                    None,
                )?;
            }

            let write_result = timeline.write_packet(&mut pipe, data, frame_count, flags);
            unsafe {
                capture.capture_client.ReleaseBuffer(frame_count)?;
            }
            write_result?;

            packet_frames = unsafe { capture.capture_client.GetNextPacketSize()? };
        }
    }

    Ok(())
}

struct LoopbackCapture {
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    spec: AudioSpec,
    silence_fill_delay_frames: u64,
}

impl LoopbackCapture {
    fn open() -> AppResult<Self> {
        let audio_client = default_render_audio_client()?;
        let mix_format = MixFormat::get(&audio_client)?;
        let spec = unsafe { AudioSpec::from_wave_format(mix_format.as_ptr())? };
        let silence_fill_delay_frames = default_silence_fill_delay_frames(&audio_client, spec)?;

        unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                AUDIO_BUFFER_100NS,
                0,
                mix_format.as_ptr(),
                None,
            )?;
        }

        let capture_client = unsafe { audio_client.GetService::<IAudioCaptureClient>()? };
        Ok(Self {
            audio_client,
            capture_client,
            spec,
            silence_fill_delay_frames,
        })
    }
}

struct AudioTimeline {
    start: Instant,
    frames_written: u64,
    silence_fill_delay_frames: u64,
    spec: AudioSpec,
}

impl AudioTimeline {
    fn new(spec: AudioSpec, silence_fill_delay_frames: u64) -> Self {
        Self {
            start: Instant::now(),
            frames_written: 0,
            silence_fill_delay_frames,
            spec,
        }
    }

    fn write_packet(
        &mut self,
        pipe: &mut File,
        data: *mut u8,
        frame_count: u32,
        flags: u32,
    ) -> AppResult<()> {
        if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
            self.write_silence_frames(pipe, frame_count as u64)?;
            return Ok(());
        }

        let byte_count = frame_count as usize * self.spec.frame_bytes;
        let samples = unsafe { slice::from_raw_parts(data, byte_count) };
        pipe.write_all(samples)?;
        self.frames_written += frame_count as u64;
        Ok(())
    }

    fn write_due_silence(&mut self, pipe: &mut File) -> AppResult<()> {
        let target_frames = self
            .elapsed_frames()
            .saturating_sub(self.silence_fill_delay_frames);
        if target_frames > self.frames_written {
            self.write_silence_frames(pipe, target_frames - self.frames_written)?;
        }
        Ok(())
    }

    fn elapsed_frames(&self) -> u64 {
        (self.start.elapsed().as_nanos() * self.spec.sample_rate as u128 / 1_000_000_000) as u64
    }

    fn write_silence_frames(&mut self, pipe: &mut File, frame_count: u64) -> AppResult<()> {
        let max_chunk_frames = self.spec.max_silence_write_frames();
        let mut remaining_frames = frame_count;

        while remaining_frames > 0 {
            let chunk_frames = remaining_frames.min(max_chunk_frames);
            let byte_count = chunk_frames as usize * self.spec.frame_bytes;
            let silence = vec![0_u8; byte_count];
            pipe.write_all(&silence)?;
            self.frames_written += chunk_frames;
            remaining_frames -= chunk_frames;
        }

        Ok(())
    }
}

fn default_silence_fill_delay_frames(
    audio_client: &IAudioClient,
    spec: AudioSpec,
) -> AppResult<u64> {
    let mut default_device_period_100ns = 0_i64;
    unsafe {
        audio_client.GetDevicePeriod(Some(&mut default_device_period_100ns), None)?;
    }

    let device_period_frames =
        default_device_period_100ns.max(0) as u64 * spec.sample_rate as u64 / 10_000_000;
    Ok((device_period_frames * 2).max(spec.sample_rate as u64 / 100))
}

fn default_render_audio_client() -> AppResult<IAudioClient> {
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None::<&IUnknown>, CLSCTX_ALL)? };
    let device: IMMDevice = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };
    let audio_client = unsafe { device.Activate::<IAudioClient>(CLSCTX_ALL, None)? };
    Ok(audio_client)
}

#[derive(Clone, Copy)]
struct AudioSpec {
    ffmpeg_sample_format: &'static str,
    sample_rate: u32,
    channels: u16,
    frame_bytes: usize,
}

impl AudioSpec {
    unsafe fn from_wave_format(format: *const WAVEFORMATEX) -> AppResult<Self> {
        if format.is_null() {
            return Err("WASAPI returned a null mix format".into());
        }

        let wave_format = unsafe { ptr::read_unaligned(format) };
        let format_tag = wave_format.wFormatTag as u32;
        let sample_rate = wave_format.nSamplesPerSec;
        let channels = wave_format.nChannels;
        let bits_per_sample = wave_format.wBitsPerSample;
        let frame_bytes = wave_format.nBlockAlign as usize;

        let subformat = if format_tag == WAVE_FORMAT_EXTENSIBLE {
            let extensible = format as *const WAVEFORMATEXTENSIBLE;
            Some(unsafe { ptr::addr_of!((*extensible).SubFormat).read_unaligned() })
        } else {
            None
        };

        let ffmpeg_sample_format = match (format_tag, subformat, bits_per_sample) {
            (WAVE_FORMAT_IEEE_FLOAT, _, 32)
            | (WAVE_FORMAT_EXTENSIBLE, Some(KSDATAFORMAT_SUBTYPE_IEEE_FLOAT), 32) => "f32le",
            (WAVE_FORMAT_PCM, _, 16)
            | (WAVE_FORMAT_EXTENSIBLE, Some(KSDATAFORMAT_SUBTYPE_PCM), 16) => "s16le",
            (WAVE_FORMAT_PCM, _, 24)
            | (WAVE_FORMAT_EXTENSIBLE, Some(KSDATAFORMAT_SUBTYPE_PCM), 24) => "s24le",
            (WAVE_FORMAT_PCM, _, 32)
            | (WAVE_FORMAT_EXTENSIBLE, Some(KSDATAFORMAT_SUBTYPE_PCM), 32) => "s32le",
            _ => {
                return Err(format!(
                    "Unsupported WASAPI mix format tag={format_tag} bits={bits_per_sample}"
                )
                .into());
            }
        };

        if sample_rate == 0 || channels == 0 || frame_bytes == 0 {
            return Err("WASAPI returned an invalid audio mix format".into());
        }

        Ok(Self {
            ffmpeg_sample_format,
            sample_rate,
            channels,
            frame_bytes,
        })
    }

    fn max_silence_write_frames(self) -> u64 {
        (self.sample_rate as u64 * MAX_SILENCE_WRITE_MS as u64 / 1_000).max(1)
    }
}

fn is_expected_pipe_close(error: &(dyn Error + 'static)) -> bool {
    let Some(io_error) = error.downcast_ref::<std::io::Error>() else {
        return false;
    };

    matches!(
        io_error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::UnexpectedEof
    ) || matches!(io_error.raw_os_error(), Some(109 | 232))
}

struct RunningAudioClient {
    audio_client: IAudioClient,
}

impl RunningAudioClient {
    fn start(audio_client: &IAudioClient) -> AppResult<Self> {
        unsafe {
            audio_client.Start()?;
        }
        Ok(Self {
            audio_client: audio_client.clone(),
        })
    }
}

impl Drop for RunningAudioClient {
    fn drop(&mut self) {
        unsafe {
            let _ = self.audio_client.Stop();
        }
    }
}

struct MixFormat(*mut WAVEFORMATEX);

impl MixFormat {
    fn get(audio_client: &IAudioClient) -> AppResult<Self> {
        let format = unsafe { audio_client.GetMixFormat()? };
        Ok(Self(format))
    }

    fn as_ptr(&self) -> *const WAVEFORMATEX {
        self.0
    }
}

impl Drop for MixFormat {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0.cast()));
        }
    }
}

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn initialize() -> AppResult<Self> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == S_OK || result == S_FALSE {
            return Ok(Self {
                should_uninitialize: true,
            });
        }

        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                should_uninitialize: false,
            });
        }

        Err(format!("Could not initialize COM for WASAPI: {result:?}").into())
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct NamedPipeServer {
    handle: Option<HANDLE>,
}

unsafe impl Send for NamedPipeServer {}

impl NamedPipeServer {
    fn new(path: &str) -> AppResult<Self> {
        let mut wide_path = path.encode_utf16().collect::<Vec<_>>();
        wide_path.push(0);

        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide_path.as_ptr()),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                AUDIO_PIPE_BUFFER_BYTES,
                AUDIO_PIPE_BUFFER_BYTES,
                0,
                None,
            )
        };

        if handle.is_invalid() {
            return Err(windows::core::Error::from_win32().into());
        }

        Ok(Self {
            handle: Some(handle),
        })
    }

    fn connect(mut self) -> AppResult<File> {
        let handle = self
            .handle
            .take()
            .ok_or("Named pipe server was already used")?;
        let connected = unsafe { ConnectNamedPipe(handle, None) };
        if let Err(error) = connected {
            if error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(error.into());
            }
        }

        let owned_handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        Ok(File::from(owned_handle))
    }
}

impl Drop for NamedPipeServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

fn unique_pipe_path() -> AppResult<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(format!(
        r"\\.\pipe\windows-rust-record-v2-audio-{}-{now}",
        std::process::id()
    ))
}
