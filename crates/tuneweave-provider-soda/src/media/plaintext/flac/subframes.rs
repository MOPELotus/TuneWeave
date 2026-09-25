//! Walk compressed subframes to find the exact footer, without allocating PCM.
//! A zero CRC remainder alone cannot locate a footer: appending zero bytes to
//! a valid frame also leaves that remainder zero.
pub(super) fn size(bytes: &[u8], samples: usize, depth: u8, assignment: u8) -> Option<usize> {
    let channels = match assignment {
        n @ 0..=7 => n + 1,
        8..=10 => 2,
        _ => return None,
    };
    let mut bits = Bits { bytes, pos: 0 };
    for channel in 0..channels {
        let side = matches!((assignment, channel), (8 | 10, 1) | (9, 0));
        let mut width = usize::from(depth) + usize::from(side);
        if bits.read(1)? != 0 {
            return None;
        }
        let kind = bits.read(6)?;
        if bits.read(1)? != 0 {
            let wasted = bits.unary((width - 2) as u32)? as usize + 1;
            width = width.checked_sub(wasted)?;
        }
        match kind {
            0 => bits.skip(width)?,
            1 => bits.skip(samples.checked_mul(width)?)?,
            8..=12 | 32..=63 => {
                let order = if kind <= 12 { kind - 8 } else { kind - 31 } as usize;
                if order >= samples {
                    return None;
                }
                bits.skip(order.checked_mul(width)?)?;
                if kind >= 32 {
                    let precision = bits.read(4)? as usize + 1;
                    if precision == 16 || bits.read(5)? >= 16 {
                        return None;
                    }
                    bits.skip(order.checked_mul(precision)?)?;
                }
                residual(&mut bits, samples, order)?;
            }
            _ => return None,
        }
    }
    let padding = (8 - bits.pos % 8) % 8;
    if bits.read(padding)? != 0 {
        return None;
    }
    Some(bits.pos / 8)
}

fn residual(bits: &mut Bits<'_>, samples: usize, order: usize) -> Option<()> {
    let parameter_bits = match bits.read(2)? {
        0 => 4,
        1 => 5,
        _ => return None,
    };
    let partitions = 1_usize << bits.read(4)?;
    let per_partition = samples / partitions;
    if samples % partitions != 0 || per_partition <= order {
        return None;
    }
    for partition in 0..partitions {
        let count = per_partition - if partition == 0 { order } else { 0 };
        let parameter = bits.read(parameter_bits)?;
        if parameter == (1 << parameter_bits) - 1 {
            let width = bits.read(5)? as usize;
            bits.skip(count.checked_mul(width)?)?;
        } else {
            for _ in 0..count {
                // Folded residuals fit u32 except u32::MAX (which would encode
                // the forbidden residual i32::MIN). No PCM reconstruction needed.
                let high = bits.unary((u32::MAX - 1) >> parameter)?;
                let low = bits.read(parameter as usize)?;
                if ((high << parameter) | low) == u32::MAX {
                    return None;
                }
            }
        }
    }
    Some(())
}

struct Bits<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn skip(&mut self, count: usize) -> Option<()> {
        let end = self.pos.checked_add(count)?;
        if end > self.bytes.len().checked_mul(8)? {
            return None;
        }
        self.pos = end;
        Some(())
    }
    fn read(&mut self, mut count: usize) -> Option<u32> {
        let mut value = 0;
        while count != 0 {
            let available = 8 - self.pos % 8;
            let take = available.min(count);
            let byte = *self.bytes.get(self.pos / 8)?;
            value = (value << take) | (u32::from(byte >> (available - take)) & ((1 << take) - 1));
            self.pos += take;
            count -= take;
        }
        Some(value)
    }
    fn unary(&mut self, limit: u32) -> Option<u32> {
        let mut zeros = 0_u32;
        loop {
            let available = 8 - self.pos % 8;
            let byte = *self.bytes.get(self.pos / 8)? << (self.pos % 8);
            let n = (byte.leading_zeros() as usize).min(available);
            zeros = zeros.checked_add(n as u32)?;
            if zeros > limit {
                return None;
            }
            self.pos += n;
            if n < available {
                self.pos += 1;
                return Some(zeros);
            }
        }
    }
}
