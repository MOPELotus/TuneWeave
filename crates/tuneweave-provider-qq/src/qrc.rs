use std::io::Read;

use flate2::read::ZlibDecoder;

const QRC_KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";
const MASK_28: u32 = 0x0fff_ffff;

const INITIAL_PERMUTATION: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6,
    64, 56, 48, 40, 32, 24, 16, 8, 57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3, 61,
    53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];
const FINAL_PERMUTATION: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31, 38, 6, 46, 14, 54, 22, 62, 30,
    37, 5, 45, 13, 53, 21, 61, 29, 36, 4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27,
    34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];
const EXPANSION: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18,
    19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];
const ROUND_PERMUTATION: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19,
    13, 30, 6, 22, 11, 4, 25,
];
const KEY_PERMUTATION_LEFT: [u8; 28] = [
    56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59,
    51, 43, 35,
];
const KEY_PERMUTATION_RIGHT: [u8; 28] = [
    62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28, 20, 12, 4,
    27, 19, 11, 3,
];
const KEY_COMPRESSION: [u8; 48] = [
    13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1, 40, 51,
    30, 36, 46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31,
];
const KEY_ROTATIONS: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
const S_BOXES: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12,
        11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9,
        1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1,
        10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15,
        4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5,
        14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6,
        9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2,
        12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10,
        10, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15,
        10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14,
        2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13,
        14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5,
        15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5,
        12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4,
        10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6,
        11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10,
        8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];

pub(crate) fn decrypt_qrc(encrypted: &str) -> std::result::Result<String, String> {
    if encrypted.is_empty() {
        return Ok(String::new());
    }
    let encrypted = hex::decode(encrypted).map_err(|error| format!("invalid QRC hex: {error}"))?;
    if encrypted.len() % 8 != 0 {
        return Err(format!(
            "QRC ciphertext length {} is not aligned to 8-byte blocks",
            encrypted.len()
        ));
    }

    let key_one: &[u8; 8] = QRC_KEY[..8].try_into().expect("fixed key segment");
    let key_two: &[u8; 8] = QRC_KEY[8..16].try_into().expect("fixed key segment");
    let key_three: &[u8; 8] = QRC_KEY[16..].try_into().expect("fixed key segment");
    let schedule_one = des_subkeys(key_one);
    let schedule_two = des_subkeys(key_two);
    let schedule_three = des_subkeys(key_three);
    let mut decrypted = Vec::with_capacity(encrypted.len());
    for block in encrypted.chunks_exact(8) {
        let block = u64::from_be_bytes(block.try_into().expect("eight-byte block"));
        let block = des_block(block, schedule_three.iter().rev().copied());
        let block = des_block(block, schedule_two.iter().copied());
        let block = des_block(block, schedule_one.iter().rev().copied());
        decrypted.extend_from_slice(&block.to_be_bytes());
    }

    let mut decoder = ZlibDecoder::new(decrypted.as_slice());
    let mut plain = Vec::new();
    decoder
        .read_to_end(&mut plain)
        .map_err(|error| format!("invalid QRC zlib stream: {error}"))?;
    String::from_utf8(plain).map_err(|error| format!("QRC text is not UTF-8: {error}"))
}

fn des_subkeys(key: &[u8; 8]) -> [u64; 16] {
    let mut left = KEY_PERMUTATION_LEFT
        .into_iter()
        .enumerate()
        .fold(0_u32, |value, (index, position)| {
            value | (u32::from(qq_key_bit(key, position)) << (31 - index))
        });
    let mut right = KEY_PERMUTATION_RIGHT
        .into_iter()
        .enumerate()
        .fold(0_u32, |value, (index, position)| {
            value | (u32::from(qq_key_bit(key, position)) << (31 - index))
        });
    let mut subkeys = [0_u64; 16];
    for (round, rotation) in KEY_ROTATIONS.into_iter().enumerate() {
        left = ((left << rotation) | (left >> (28 - rotation))) & (MASK_28 << 4);
        right = ((right << rotation) | (right >> (28 - rotation))) & (MASK_28 << 4);
        let mut subkey = 0_u64;
        for (index, position) in KEY_COMPRESSION.into_iter().enumerate() {
            let bit = if index < 24 {
                (left >> (31 - position)) & 1
            } else {
                // QQ's historical QRC codec selects the D half one position to the right of
                // standard DES PC-2. Its last selection therefore reaches the zero padding bit.
                (right >> (31 - (position - 27))) & 1
            };
            subkey = (subkey << 1) | u64::from(bit);
        }
        subkeys[round] = subkey;
    }
    subkeys
}

fn qq_key_bit(key: &[u8; 8], position: u8) -> u8 {
    let word = usize::from(position / 32);
    let within_word = position % 32;
    let byte = word * 4 + 3 - usize::from(within_word / 8);
    (key[byte] >> (7 - within_word % 8)) & 1
}

fn des_block(block: u64, subkeys: impl IntoIterator<Item = u64>) -> u64 {
    let block = reverse_word_bytes(block);
    reverse_word_bytes(des_block_standard(block, subkeys))
}

fn des_block_standard(block: u64, subkeys: impl IntoIterator<Item = u64>) -> u64 {
    let block = permute(block, 64, &INITIAL_PERMUTATION);
    let mut left = (block >> 32) as u32;
    let mut right = block as u32;
    for subkey in subkeys {
        (left, right) = (right, left ^ feistel(right, subkey));
    }
    permute(
        (u64::from(right) << 32) | u64::from(left),
        64,
        &FINAL_PERMUTATION,
    )
}

fn reverse_word_bytes(value: u64) -> u64 {
    (u64::from(((value >> 32) as u32).swap_bytes()) << 32) | u64::from((value as u32).swap_bytes())
}

fn feistel(right: u32, subkey: u64) -> u32 {
    let expanded = permute(u64::from(right), 32, &EXPANSION) ^ subkey;
    let mut substituted = 0_u32;
    for (index, s_box) in S_BOXES.iter().enumerate() {
        let value = ((expanded >> (42 - index * 6)) & 0x3f) as u8;
        let row = ((value & 0x20) >> 4) | (value & 1);
        let column = (value >> 1) & 0x0f;
        substituted = (substituted << 4) | u32::from(s_box[usize::from(row * 16 + column)]);
    }
    permute(u64::from(substituted), 32, &ROUND_PERMUTATION) as u32
}

fn permute(input: u64, input_width: u8, table: &[u8]) -> u64 {
    table.iter().fold(0_u64, |output, position| {
        (output << 1) | ((input >> (input_width - position)) & 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_independent_qrc_vector() {
        let schedule = des_subkeys(QRC_KEY[16..].try_into().expect("third key"));
        assert_eq!(schedule[15], 0xf03a_0018_caa2);
        assert_eq!(schedule[0], 0x2018_2600_682c);
        assert_eq!(
            des_block(0x32da_bb4c_5e98_46fa, schedule.iter().rev().copied()),
            0x2923_1c3f_f677_b640
        );
        let encrypted =
            "32DABB4C5E9846FA00500DAD626F835FEAF8FC836D78403DAE4DC6ACD55A35D4C56A9C418E42B35F";
        assert_eq!(
            decrypt_qrc(encrypted).expect("decrypt QRC vector"),
            "[00:01.00]TuneWeave QRC 测试\n"
        );
    }

    #[test]
    fn rejects_malformed_qrc_before_decompression() {
        assert!(decrypt_qrc("not-hex").is_err());
        assert!(decrypt_qrc("00").is_err());
    }
}
