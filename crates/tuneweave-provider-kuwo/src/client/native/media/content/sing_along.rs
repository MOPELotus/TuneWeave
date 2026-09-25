//! Bounded conversion of the independently authorized BCMS stem resource.
use super::*;
use lewton::inside_ogg::OggStreamReader;
use std::io::Cursor;

mod mix;
mod ogg;
#[cfg(test)]
pub(crate) mod tests;

const WAV_HEADER: usize = 44;
const MAX_WAV_BYTES: usize = 512 * 1024 * 1024;
const MAX_FRAMES: u64 = ((MAX_WAV_BYTES - WAV_HEADER) / 4) as u64;
const SAMPLE_RATE: u32 = 44_100;

pub(super) fn convert(bytes: &[u8], control: &Control) -> Result<Vec<u8>> {
    let source = ogg::inspect(bytes, control)?;
    let mut reader = OggStreamReader::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if reader.ident_hdr.audio_channels != 4
        || reader.ident_hdr.audio_sample_rate != SAMPLE_RATE
        || reader.stream_serial() != source.serial
    {
        return Err(invalid());
    }
    let frames = usize::try_from(source.frames).map_err(|_| invalid())?;
    let length = WAV_HEADER + frames * 4;
    let mut output = Vec::with_capacity(length.min(64 * 1024));
    output.extend_from_slice(&wave_header((frames * 4) as u32));
    let mut mixer = mix::Mixer::new();
    let mut decoded = 0;
    let mut fed = 0;
    loop {
        control.check()?;
        let Some(mut packet) = reader.read_dec_packet_itl().map_err(|_| invalid())? else {
            break;
        };
        if packet.len() % 4 != 0
            || packet.len() > 8192 * 4
            || reader.stream_serial() != source.serial
        {
            return Err(invalid());
        }
        if packet.len() / 4 > frames - decoded {
            // A one-page audio stream leaves Lewton's preceding granule unset.
            // Only its final page may trim codec padding to the verified EOS.
            if reader.get_last_absgp() != Some(source.frames) {
                return Err(invalid());
            }
            packet.truncate((frames - decoded) * 4);
        }
        for samples in packet.chunks_exact(4) {
            let mixed = mixer.push(samples);
            append_frame(&mut output, mixed, fed, frames);
            fed += 1;
        }
        decoded += packet.len() / 4;
    }
    if decoded != frames || reader.get_last_absgp() != Some(source.frames) {
        return Err(invalid());
    }
    // Native blocks contain 512 input frames; flush clears rather than drains.
    // Feed silence through the same recurrence, discard its measured lookahead,
    // and retain exactly the source granule count, including a short final block.
    let drained = (frames + mix::LATENCY_FRAMES).div_ceil(mix::BLOCK_FRAMES) * mix::BLOCK_FRAMES;
    while fed < drained {
        control.check()?;
        let mixed = mixer.push(&[0; 4]);
        append_frame(&mut output, mixed, fed, frames);
        fed += 1;
    }
    if output.len() != length {
        return Err(invalid());
    }
    Ok(output)
}
fn append_frame(output: &mut Vec<u8>, samples: [f32; 2], fed: usize, frames: usize) {
    if fed >= mix::LATENCY_FRAMES && fed - mix::LATENCY_FRAMES < frames {
        for sample in samples {
            let pcm = (sample * 32_768.).round().clamp(-32_768., 32_767.) as i16;
            output.extend_from_slice(&pcm.to_le_bytes());
        }
    }
}
fn wave_header(data_bytes: u32) -> [u8; WAV_HEADER] {
    let mut header = [0; WAV_HEADER];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(data_bytes + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&2_u16.to_le_bytes());
    header[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
    header[32..34].copy_from_slice(&4_u16.to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..].copy_from_slice(&data_bytes.to_le_bytes());
    header
}
