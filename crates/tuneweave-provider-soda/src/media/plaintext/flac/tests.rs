use super::*;

#[derive(Default)]
struct Writer(Vec<bool>);
impl Writer {
    fn put(&mut self, value: u64, count: usize) {
        self.0
            .extend((0..count).rev().map(|n| value & (1 << n) != 0));
    }
    fn bytes(mut self) -> Vec<u8> {
        while self.0.len() % 8 != 0 {
            self.0.push(false);
        }
        self.0
            .chunks_exact(8)
            .map(|chunk| chunk.iter().fold(0, |v, b| (v << 1) | u8::from(*b)))
            .collect()
    }
}

fn checksum(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0_u16, |crc, b| {
        (crc << 8) ^ CRC16[usize::from((crc >> 8) as u8 ^ b)]
    })
}
fn frame(body: &[u8], number: u8, variable: bool, assignment: u8) -> Vec<u8> {
    let mut out = vec![
        255,
        0xf8 | u8::from(variable),
        0x69,
        assignment << 4,
        number,
        63,
    ];
    out.push(crc8(&out));
    out.extend_from_slice(body);
    out.extend_from_slice(&checksum(&out).to_be_bytes());
    out
}
fn file(frames: &[Vec<u8>], total: u64, depth: u8, channels: u8) -> Vec<u8> {
    let mut out = b"fLaC\x80\0\0\x22".to_vec();
    let mut info = [0_u8; 34];
    info[..4].copy_from_slice(&[0, 64, 0, 64]);
    let packed =
        (44100_u64 << 44) | (u64::from(channels - 1) << 41) | (u64::from(depth - 1) << 36) | total;
    info[10..18].copy_from_slice(&packed.to_be_bytes());
    out.extend_from_slice(&info);
    for f in frames {
        out.extend_from_slice(f);
    }
    out
}
// Independent, all-zero PCM encodings described by RFC 9639 sections 9.2/9.3.
// kind = constant/verbatim/fixed/LPC; residual = Rice4/Rice5/escaped zero width.
fn body(kind: u8, residual: u8, depth: u8, assignment: u8, wasted: bool) -> Vec<u8> {
    let mut b = Writer::default();
    let channels = if assignment <= 7 { assignment + 1 } else { 2 };
    for channel in 0..channels {
        let width = usize::from(depth)
            + usize::from(matches!((assignment, channel), (8 | 10, 1) | (9, 0)))
            - usize::from(wasted);
        b.put(u64::from(kind) << 1 | u64::from(wasted), 8);
        if wasted {
            b.put(1, 1);
        }
        match kind {
            0 => b.put(0, width),
            1 => {
                for _ in 0..64 {
                    b.put(0, width);
                }
            }
            _ => {
                let order = if kind < 32 { kind - 8 } else { kind - 31 };
                for _ in 0..order {
                    b.put(0, width);
                }
                if kind >= 32 {
                    b.put(0, 4);
                    b.put(0, 5);
                    b.put(0, usize::from(order));
                }
                b.put(u64::from(residual == 1), 2);
                b.put(0, 4);
                if residual == 2 {
                    b.put(15, 4);
                    b.put(0, 5);
                } else {
                    b.put(0, if residual == 1 { 5 } else { 4 });
                    for _ in usize::from(order)..64 {
                        b.put(1, 1);
                    }
                }
            }
        }
    }
    b.bytes()
}

#[test]
fn plaintext_flac_accepts_all_subframe_kinds_rice_modes_and_stereo_assignments() {
    for (case, (kind, residual, depth, assignment, wasted)) in [
        (0, 0, 8, 0, false),
        (1, 0, 16, 1, false),
        (8, 0, 16, 0, false),
        (12, 1, 24, 1, false),
        (33, 2, 16, 0, false),
        (63, 0, 16, 0, false),
        (0, 0, 32, 8, true),
        (1, 0, 32, 9, true),
        (12, 1, 24, 10, true),
        (0, 0, 16, 7, false),
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = file(
            &[frame(
                &body(kind, residual, depth, assignment, wasted),
                0,
                false,
                assignment,
            )],
            64,
            depth,
            if assignment <= 7 { assignment + 1 } else { 2 },
        );
        assert_eq!(inspect(&bytes), Some(1), "case {case}");
        if let Some(dir) = std::env::var_os("TUNEWEAVE_SODA_SYNTHETIC_OUTPUT_DIR") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("flac-framing-{case}.flac")),
                &bytes,
            )
            .unwrap();
        }
    }
}

#[test]
fn plaintext_flac_rejects_malformed_subframes_even_with_correct_frame_crc() {
    let mut bad = vec![vec![0x80, 0, 0], vec![4, 0, 0], vec![1, 0, 0]];
    // Invalid residual method and partitions that cannot hold predictor warm-up.
    for (method, partition) in [(2, 0), (3, 0), (0, 7), (0, 6)] {
        let mut b = Writer::default();
        b.put(18, 8);
        b.put(0, 16);
        b.put(method, 2);
        b.put(partition, 4);
        b.put(0, 16);
        bad.push(b.bytes());
    }
    // LPC coefficient precision 16 and a negative prediction shift are forbidden.
    for (precision, shift) in [(15, 0), (0, 31)] {
        let mut b = Writer::default();
        b.put(64, 8);
        b.put(0, 16);
        b.put(precision, 4);
        b.put(shift, 5);
        b.put(0, 16);
        bad.push(b.bytes());
    }
    let mut extra_zero = body(0, 0, 16, 0, false);
    extra_zero.push(0); // Correct CRC cannot turn trailing bytes into a subframe.
    bad.push(extra_zero);
    let mut padding = body(8, 0, 16, 0, true);
    *padding.last_mut().unwrap() |= 1;
    bad.push(padding);
    for (case, b) in bad.into_iter().enumerate() {
        let f = frame(&b, 0, false, 0);
        assert_eq!(checksum(&f), 0);
        assert_eq!(inspect(&file(&[f], 64, 16, 1)), None, "case {case}");
    }
}

#[test]
fn plaintext_flac_checks_numbering_and_exact_end_even_when_total_is_unknown() {
    let b = body(0, 0, 16, 0, false);
    for variable in [false, true] {
        let f = frame(&b, 0, variable, 0);
        let second = frame(&b, if variable { 64 } else { 1 }, variable, 0);
        for total in [0, 128] {
            let good = file(&[f.clone(), second.clone()], total, 16, 1);
            assert_eq!(inspect(&good), Some(2));
            for suffix in [vec![0], vec![0; 128], f.clone()] {
                let mut bytes = good.clone();
                bytes.extend_from_slice(&suffix);
                assert!(inspect(&bytes).is_none());
            }
        }
        assert!(inspect(&file(&[f.clone(), frame(&b, 2, variable, 0)], 128, 16, 1)).is_none());
        assert!(inspect(&file(&[f, frame(&b, 1, !variable, 0)], 128, 16, 1)).is_none());
    }
}
