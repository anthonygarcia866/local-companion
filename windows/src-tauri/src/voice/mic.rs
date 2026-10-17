// The microphone, into memory only (voice plan §1): open only while the
// push-to-talk key is held, 16 kHz mono 16-bit (Windows' wave mapper converts
// from the device's own format), polled every 30 ms. The samples live in a
// Vec until they are transcribed, then they're dropped; nothing is written.
//
// waveIn (winmm) rather than WASAPI: the same audio for dictation, a fraction
// of the code, and no new crate (a feature of the `windows` crate Glim has).

use windows::Win32::Media::Audio::{
    waveInAddBuffer, waveInClose, waveInGetNumDevs, waveInOpen, waveInPrepareHeader, waveInReset, waveInStart,
    waveInUnprepareHeader, CALLBACK_NULL, HWAVEIN, WAVEFORMATEX, WAVEHDR, WAVE_FORMAT_PCM, WAVE_MAPPER, WHDR_DONE,
};

use super::SAMPLE_RATE;

/// 100 ms per buffer.
const CHUNK: usize = (SAMPLE_RATE / 10) as usize;
const BUFFERS: usize = 6;
/// How often the buffers (and the caller's stop) are looked at.
pub const POLL_MS: u64 = 30;

/// Why there's no recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicError {
    /// No input device at all.
    NoDevice,
    /// The device couldn't be opened (in use, blocked by privacy settings).
    Open,
}

impl MicError {
    pub fn code(self) -> &'static str {
        match self {
            MicError::NoDevice => "no-mic",
            MicError::Open => "mic-blocked",
        }
    }
}

/// Records until `stop()` says so (polled every POLL_MS), calling `level`
/// for each 100 ms chunk. Returns the samples.
pub fn record(mut stop: impl FnMut() -> bool, mut level: impl FnMut(f32)) -> Result<Vec<i16>, MicError> {
    if unsafe { waveInGetNumDevs() } == 0 {
        return Err(MicError::NoDevice);
    }
    let fmt = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let mut handle = HWAVEIN::default();
    if unsafe { waveInOpen(Some(&mut handle), WAVE_MAPPER, &fmt, None, None, CALLBACK_NULL) } != 0 {
        return Err(MicError::Open);
    }
    // Buffers and headers stay put (boxed) while the driver holds them.
    let mut data: Vec<Box<[i16]>> = (0..BUFFERS).map(|_| vec![0i16; CHUNK].into_boxed_slice()).collect();
    let mut headers: Vec<Box<WAVEHDR>> = Vec::with_capacity(BUFFERS);
    let size = std::mem::size_of::<WAVEHDR>() as u32;
    for buf in data.iter_mut() {
        let mut h = Box::new(WAVEHDR {
            lpData: windows::core::PSTR(buf.as_mut_ptr() as *mut u8),
            dwBufferLength: (CHUNK * 2) as u32,
            ..Default::default()
        });
        unsafe {
            waveInPrepareHeader(handle, h.as_mut(), size);
            waveInAddBuffer(handle, h.as_mut(), size);
        }
        headers.push(h);
    }
    unsafe { waveInStart(handle) };
    let mut out: Vec<i16> = Vec::with_capacity(SAMPLE_RATE as usize * 5);
    let collect = |headers: &mut Vec<Box<WAVEHDR>>, requeue: bool, out: &mut Vec<i16>, level: &mut dyn FnMut(f32)| {
        for h in headers.iter_mut() {
            if h.dwFlags & WHDR_DONE == 0 {
                continue;
            }
            let n = h.dwBytesRecorded as usize / 2;
            let chunk = unsafe { std::slice::from_raw_parts(h.lpData.0 as *const i16, n) };
            out.extend_from_slice(chunk);
            level(super::level(chunk));
            h.dwFlags &= !WHDR_DONE;
            h.dwBytesRecorded = 0;
            if requeue {
                unsafe { waveInAddBuffer(handle, h.as_mut(), size) };
            }
        }
    };
    let max = super::MAX_HELD_MS / POLL_MS;
    for _ in 0..max {
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        collect(&mut headers, true, &mut out, &mut level);
        if stop() {
            break;
        }
    }
    unsafe { waveInReset(handle) };
    collect(&mut headers, false, &mut out, &mut level);
    for h in headers.iter_mut() {
        unsafe { waveInUnprepareHeader(handle, h.as_mut(), size) };
    }
    unsafe { waveInClose(handle) };
    drop(data);
    Ok(out)
}
