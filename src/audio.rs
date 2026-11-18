use crate::dsp::{SpectrumAnalyzer, NUM_BARS};
use std::mem::size_of;
use std::ptr::{copy_nonoverlapping, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{implement, Interface, HRESULT, PROPVARIANT};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

const WAVE_FORMAT_PCM_TAG: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const KSDATAFORMAT_SUBTYPE_PCM_GUID: windows::core::GUID =
    windows::core::GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AudioTargetState {
    is_playing: bool,
    pid: Option<u32>,
    shutdown: bool,
}

/// Lock-free 10-band spectrum levels shared with the UI render thread.
pub struct AudioHandle {
    bars: [AtomicU32; NUM_BARS],
    active: AtomicBool,
    reconnect_requested: AtomicBool,
    state: Mutex<AudioTargetState>,
    cv: Condvar,
}

impl AudioHandle {
    pub fn read_bars(&self) -> [f32; NUM_BARS] {
        let mut out = [0.0f32; NUM_BARS];
        for (i, slot) in self.bars.iter().enumerate() {
            out[i] = f32::from_bits(slot.load(Ordering::Relaxed));
        }
        out
    }

    pub fn is_decaying_or_active(&self) -> bool {
        if self.active.load(Ordering::Relaxed) {
            return true;
        }
        self.read_bars().iter().any(|&v| v > 0.005)
    }

    pub fn set_target(&self, is_playing: bool, pid: Option<u32>) {
        if let Ok(mut guard) = self.state.lock() {
            if is_playing {
                self.active.store(true, Ordering::Relaxed);
            }
            if guard.is_playing != is_playing || guard.pid != pid {
                guard.is_playing = is_playing;
                guard.pid = pid;
                self.cv.notify_all();
            }
        }
    }

    pub fn notify_device_changed(&self) {
        self.reconnect_requested.store(true, Ordering::Relaxed);
        self.cv.notify_all();
    }

    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.state.lock() {
            guard.shutdown = true;
            guard.is_playing = false;
            self.cv.notify_all();
        }
    }

    fn write_bars(&self, levels: &[f32; NUM_BARS]) {
        for (slot, &val) in self.bars.iter().zip(levels.iter()) {
            slot.store(val.to_bits(), Ordering::Relaxed);
        }
    }
}

pub fn start_audio_worker() -> Arc<AudioHandle> {
    let handle = Arc::new(AudioHandle {
        bars: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
        active: AtomicBool::new(false),
        reconnect_requested: AtomicBool::new(false),
        state: Mutex::new(AudioTargetState {
            is_playing: false,
            pid: None,
            shutdown: false,
        }),
        cv: Condvar::new(),
    });

    let thread_handle = Arc::clone(&handle);
    std::thread::Builder::new()
        .name("mediabar-audio".into())
        .spawn(move || run_audio_loop(thread_handle))
        .expect("failed to spawn audio worker thread");

    handle
}

/// Non-owning VT_BLOB PROPVARIANT layout on x64 Windows (`vt = 65`) pointing at stack activation params.
/// Avoids `windows_core::PROPVARIANT`'s `Drop` calling `PropVariantClear` on stack memory.
#[repr(C)]
struct RawBlobPropVariant {
    vt: u16,
    w_reserved1: u16,
    w_reserved2: u16,
    w_reserved3: u16,
    cb_size: u32,
    _pad: u32,
    p_blob_data: *const u8,
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    event: HANDLE,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _activateoperation: Option<&IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        unsafe {
            let _ = SetEvent(self.event);
        }
        Ok(())
    }
}

struct OwnedEvent(HANDLE);

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct CoTaskWaveFormat(*mut WAVEFORMATEX);

impl Drop for CoTaskWaveFormat {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SampleKind {
    Float32,
    Pcm16,
}

struct CaptureSession {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    channels: usize,
    sample_rate: u32,
    sample_kind: SampleKind,
    is_process_loopback: bool,
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn activate_process_loopback(pid: u32) -> Option<CaptureSession> {
    if pid == 0 {
        return None;
    }

    unsafe {
        let event = CreateEventW(None, true, false, None).ok()?;
        let _event_guard = OwnedEvent(event);

        let params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };

        let raw_prop = RawBlobPropVariant {
            vt: 65, // VT_BLOB
            w_reserved1: 0,
            w_reserved2: 0,
            w_reserved3: 0,
            cb_size: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            _pad: 0,
            p_blob_data: (&params as *const AUDIOCLIENT_ACTIVATION_PARAMS).cast::<u8>(),
        };

        let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler { event }.into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some((&raw_prop as *const RawBlobPropVariant).cast::<PROPVARIANT>()),
            &handler,
        )
        .ok()?;

        if WaitForSingleObject(event, 2000) != WAIT_OBJECT_0 {
            return None;
        }

        let mut hr = HRESULT(0);
        let mut unknown = None;
        op.GetActivateResult(&mut hr, &mut unknown).ok()?;
        if hr.is_err() {
            return None;
        }
        let client: IAudioClient = unknown?.cast().ok()?;

        // Process Loopback virtual device does not implement GetMixFormat(); provide standard 48kHz stereo float.
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_IEEE_FLOAT_TAG,
            nChannels: 2,
            nSamplesPerSec: 48_000,
            nAvgBytesPerSec: 48_000 * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };

        let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 200_000, 0, &fmt, None)
            .ok()?;

        let capture: IAudioCaptureClient = client.GetService().ok()?;
        client.Start().ok()?;

        Some(CaptureSession {
            client,
            capture,
            channels: 2,
            sample_rate: 48_000,
            sample_kind: SampleKind::Float32,
            is_process_loopback: true,
        })
    }
}

fn activate_endpoint_loopback() -> Option<CaptureSession> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
        let mix_ptr = client.GetMixFormat().ok()?;
        let _mix_guard = CoTaskWaveFormat(mix_ptr);

        let wf = *mix_ptr;
        let channels = (wf.nChannels as usize).max(1);
        let sample_rate = wf.nSamplesPerSec.max(8000);

        let sample_kind = match wf.wFormatTag {
            WAVE_FORMAT_IEEE_FLOAT_TAG if wf.wBitsPerSample == 32 => SampleKind::Float32,
            WAVE_FORMAT_PCM_TAG if wf.wBitsPerSample == 16 => SampleKind::Pcm16,
            WAVE_FORMAT_EXTENSIBLE_TAG if wf.cbSize >= 22 => {
                let ext = *(mix_ptr.cast::<WAVEFORMATEXTENSIBLE>());
                let subformat = ext.SubFormat;
                if subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT_GUID && wf.wBitsPerSample == 32 {
                    SampleKind::Float32
                } else if subformat == KSDATAFORMAT_SUBTYPE_PCM_GUID && wf.wBitsPerSample == 16 {
                    SampleKind::Pcm16
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                300_000,
                0,
                mix_ptr,
                None,
            )
            .ok()?;
        let capture: IAudioCaptureClient = client.GetService().ok()?;
        client.Start().ok()?;

        Some(CaptureSession {
            client,
            capture,
            channels,
            sample_rate,
            sample_kind,
            is_process_loopback: false,
        })
    }
}

fn read_capture_packets(
    session: &CaptureSession,
    analyzer: &mut SpectrumAnalyzer,
    mono_scratch: &mut Vec<f32>,
) -> Result<(bool, f32), ()> {
    const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
    let mut got_audio = false;
    let mut max_amp = 0.0f32;

    unsafe {
        loop {
            let packet_frames = match session.capture.GetNextPacketSize() {
                Ok(n) => n,
                Err(_) => return Err(()),
            };
            if packet_frames == 0 {
                break;
            }

            let mut data_ptr: *mut u8 = null_mut();
            let mut frames_read = 0u32;
            let mut flags = 0u32;
            if session
                .capture
                .GetBuffer(&mut data_ptr, &mut frames_read, &mut flags, None, None)
                .is_err()
            {
                return Err(());
            }

            let frames = frames_read as usize;
            mono_scratch.clear();

            if (flags & AUDCLNT_BUFFERFLAGS_SILENT) != 0 || data_ptr.is_null() {
                mono_scratch.resize(frames, 0.0);
            } else {
                let ch = session.channels;
                let inv_ch = 1.0 / (ch as f32);
                mono_scratch.reserve(frames);
                match session.sample_kind {
                    SampleKind::Float32 => {
                        let mut floats = vec![0.0f32; frames * ch];
                        copy_nonoverlapping(data_ptr.cast::<f32>(), floats.as_mut_ptr(), frames * ch);
                        for frame in floats.chunks_exact(ch) {
                            let mut sum = 0.0f32;
                            for &s in frame {
                                sum += s;
                                let abs = s.abs();
                                if abs > max_amp {
                                    max_amp = abs;
                                }
                            }
                            mono_scratch.push(sum * inv_ch);
                        }
                    }
                    SampleKind::Pcm16 => {
                        let mut ints = vec![0i16; frames * ch];
                        copy_nonoverlapping(data_ptr.cast::<i16>(), ints.as_mut_ptr(), frames * ch);
                        let inv_i16 = 1.0 / 32768.0;
                        for frame in ints.chunks_exact(ch) {
                            let mut sum = 0.0f32;
                            for &s in frame {
                                let fs = f32::from(s) * inv_i16;
                                sum += fs;
                                let abs = fs.abs();
                                if abs > max_amp {
                                    max_amp = abs;
                                }
                            }
                            mono_scratch.push(sum * inv_ch);
                        }
                    }
                }
            }

            let _ = session.capture.ReleaseBuffer(frames_read);
            if !mono_scratch.is_empty() {
                got_audio = true;
                analyzer.push_mono_samples(mono_scratch);
            }
        }
    }

    Ok((got_audio, max_amp))
}

fn run_audio_loop(handle: Arc<AudioHandle>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let mut analyzer = SpectrumAnalyzer::new(48_000);
    let mut mono_scratch = Vec::with_capacity(2048);
    let mut session: Option<CaptureSession> = None;
    let mut active_pid: Option<u32> = None;
    let mut force_endpoint_fallback_for_pid: Option<u32> = None;
    let mut last_nonzero_audio = Instant::now();
    let mut last_tick = Instant::now();

    loop {
        let target = {
            let mut guard = match handle.state.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            while !guard.is_playing && !guard.shutdown {
                session = None;
                active_pid = None;
                handle.active.store(false, Ordering::Relaxed);

                // Decay smoothly down to zero before sleeping completely
                if handle.read_bars().iter().any(|&v| v > 0.004) {
                    drop(guard);
                    let dt = last_tick.elapsed().as_secs_f32().clamp(0.005, 0.05);
                    last_tick = Instant::now();
                    analyzer.decay_to_zero(dt);
                    handle.write_bars(&analyzer.bars());
                    std::thread::sleep(Duration::from_millis(16));
                    guard = match handle.state.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    continue;
                }

                analyzer.reset();
                handle.write_bars(&[0.0; NUM_BARS]);
                guard = match handle.cv.wait(guard) {
                    Ok(g) => g,
                    Err(_) => return,
                };
                last_tick = Instant::now();
            }
            *guard
        };

        if target.shutdown {
            break;
        }

        handle.active.store(true, Ordering::Relaxed);

        if handle.reconnect_requested.swap(false, Ordering::Relaxed) {
            drop(session.take());
            force_endpoint_fallback_for_pid = None;
        }

        // Recreate capture session if source PID changed or session not active yet.
        if target.pid != active_pid || session.is_none() {
            drop(session.take());
            active_pid = target.pid;
            last_nonzero_audio = Instant::now();
            if force_endpoint_fallback_for_pid != target.pid {
                force_endpoint_fallback_for_pid = None;
            }

            let want_process_loopback =
                target.pid.is_some() && force_endpoint_fallback_for_pid != target.pid;

            session = if want_process_loopback {
                target
                    .pid
                    .and_then(activate_process_loopback)
                    .or_else(activate_endpoint_loopback)
            } else {
                activate_endpoint_loopback()
            };

            if let Some(ref s) = session {
                analyzer.set_sample_rate(s.sample_rate);
            }
        }

        let Some(ref active_session) = session else {
            // Audio device unavailable momentarily -> decay gently and retry slowly.
            let dt = last_tick.elapsed().as_secs_f32().clamp(0.01, 0.1);
            last_tick = Instant::now();
            analyzer.decay_to_zero(dt);
            handle.write_bars(&analyzer.bars());
            std::thread::sleep(Duration::from_millis(120));
            continue;
        };

        match read_capture_packets(active_session, &mut analyzer, &mut mono_scratch) {
            Ok((got_audio, max_amp)) => {
                if max_amp > 1e-4 {
                    last_nonzero_audio = Instant::now();
                } else if active_session.is_process_loopback
                    && last_nonzero_audio.elapsed() > Duration::from_millis(2500)
                {
                    // Process loopback stayed silent for 2.5s while SMTC reports Playing
                    // (e.g., player delegates audio to an external system audio service).
                    force_endpoint_fallback_for_pid = active_pid;
                    session = None;
                    continue;
                }

                let dt = last_tick.elapsed().as_secs_f32().clamp(0.005, 0.05);
                last_tick = Instant::now();
                if !got_audio {
                    analyzer.decay_to_zero(dt * 0.65);
                }
                handle.write_bars(&analyzer.bars());
            }
            Err(()) => {
                // Default endpoint changed or device invalidated; drop and re-acquire cleanly.
                session = None;
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        }

        std::thread::sleep(Duration::from_millis(12));
    }

    drop(session);
    unsafe {
        CoUninitialize();
    }
}
