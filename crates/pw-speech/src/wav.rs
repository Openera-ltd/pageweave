//! RIFF/WAVE reading and writing (16-bit PCM), and simple resampling.

use pw_core::{ErrorCategory, PwError, PwResult};

/// A mono 16-bit PCM WAV file.
pub fn encode(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

fn bad(what: &str) -> PwError {
    PwError::new(ErrorCategory::EngineFailure, "speech.audio", format!("The speech audio is not usable ({what})."))
}

/// Read a PCM WAV (8- or 16-bit, any channel count; channels are averaged) as mono 16-bit.
pub fn decode(bytes: &[u8]) -> PwResult<(u32, Vec<i16>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(bytes, pos + 4) as usize;
        let body = pos + 8;
        // Streamed files may give a data length beyond the end: take what is there.
        let end = body.saturating_add(len).min(bytes.len());
        if id == b"fmt " {
            if end - body < 16 {
                return Err(bad("short format"));
            }
            let mut tag = u16_at(bytes, body);
            if tag == 0xFFFE && end - body >= 26 {
                tag = u16_at(bytes, body + 24); // WAVE_FORMAT_EXTENSIBLE: sub-format
            }
            fmt = Some((tag, u16_at(bytes, body + 2), u32_at(bytes, body + 4), u16_at(bytes, body + 14)));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = fmt.ok_or_else(|| bad("data before format"))?;
            if tag != 1 || channels == 0 || rate == 0 || !(bits == 16 || bits == 8) {
                return Err(bad("not 8- or 16-bit PCM"));
            }
            let data = &bytes[body..end];
            let ch = channels as usize;
            let frame = ch * (bits as usize / 8);
            let mut out = Vec::with_capacity(data.len() / frame);
            for f in data.chunks_exact(frame) {
                let sum: i32 = (0..ch)
                    .map(|c| {
                        if bits == 16 {
                            i16::from_le_bytes([f[2 * c], f[2 * c + 1]]) as i32
                        } else {
                            (f[c] as i32 - 128) << 8
                        }
                    })
                    .sum();
                out.push((sum / ch as i32) as i16);
            }
            return Ok((rate, out));
        }
        pos = body.saturating_add(len + (len & 1));
    }
    Err(bad("no audio data"))
}

/// Linear resampling (only used if an engine changes rate between sentences).
pub fn resample(samples: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == to || samples.is_empty() || from == 0 || to == 0 {
        return samples.to_vec();
    }
    let n = (samples.len() as u64 * to as u64 / from as u64) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * from as f64 / to as f64;
            let k = x.floor() as usize;
            let t = x - k as f64;
            let a = samples[k.min(samples.len() - 1)] as f64;
            let b = samples[(k + 1).min(samples.len() - 1)] as f64;
            (a + (b - a) * t).round() as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_odd_files() {
        let s: Vec<i16> = (0..1000).map(|i| ((i * 37) % 2000 - 1000) as i16).collect();
        let w = encode(22_050, &s);
        assert_eq!(w.len(), 44 + 2000);
        assert_eq!(decode(&w).expect("decode"), (22_050, s.clone()));
        // A stereo file is averaged; an extra chunk before the data is skipped.
        let mut st = Vec::new();
        st.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        st.extend_from_slice(&16u32.to_le_bytes());
        st.extend_from_slice(&[1, 0, 2, 0]);
        st.extend_from_slice(&8000u32.to_le_bytes());
        st.extend_from_slice(&32000u32.to_le_bytes());
        st.extend_from_slice(&[4, 0, 16, 0]);
        st.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        st.extend_from_slice(b"data");
        st.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // streamed: unknown length
        for (l, r) in [(100i16, 300i16), (-200, -400)] {
            st.extend_from_slice(&l.to_le_bytes());
            st.extend_from_slice(&r.to_le_bytes());
        }
        assert_eq!(decode(&st).expect("stereo"), (8000, vec![200, -300]));
        assert!(decode(b"RIFF").is_err());
        assert!(decode(b"RIFF\0\0\0\0WAVEdata\0\0\0\0").is_err());
    }

    #[test]
    fn resampling_keeps_duration() {
        let s: Vec<i16> = vec![0, 100, 200, 300];
        assert_eq!(resample(&s, 8000, 16000).len(), 8);
        assert_eq!(resample(&s, 16000, 8000), vec![0, 200]);
        assert_eq!(resample(&s, 8000, 8000), s);
    }
}
