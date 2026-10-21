// SPDX-License-Identifier: MIT
//! CarLife 2.0 public challenge calculation. No clock, keys, or pairing state.

/// Return the 32-byte lowercase hexadecimal MD5 response expected by the
/// CarLife 2.0 challenge profile. Errors intentionally contain no input data.
pub fn response(challenge: &str, identity: &str) -> Result<Vec<u8>, String> {
    if challenge.len() > 1024 || !challenge.starts_with("2.0;") {
        return Err("unsupported or oversized CarLife challenge".into());
    }
    let nonce = challenge.split(';').nth(1).filter(|value| !value.is_empty())
        .ok_or("CarLife challenge nonce is empty")?;
    let mut input = Vec::new();
    for value in ["CARLIFE", identity, nonce] { modified_utf8(value, &mut input); }
    let digest = md5(&input);
    let hex = b"0123456789abcdef";
    let mut output = Vec::with_capacity(32);
    for byte in digest {
        output.push(hex[(byte >> 4) as usize]);
        output.push(hex[(byte & 15) as usize]);
    }
    Ok(output)
}

// JNI GetStringUTFChars uses modified UTF-8: encode UTF-16 code units,
// including each surrogate separately, and escape NUL as C0 80.
fn modified_utf8(value: &str, output: &mut Vec<u8>) {
    for unit in value.encode_utf16() {
        match unit {
            1..=0x7f => output.push(unit as u8),
            0..=0x7ff => {
                output.push((0xc0 | (unit >> 6)) as u8);
                output.push((0x80 | (unit & 0x3f)) as u8);
            }
            _ => {
                output.push((0xe0 | (unit >> 12)) as u8);
                output.push((0x80 | ((unit >> 6) & 0x3f)) as u8);
                output.push((0x80 | (unit & 0x3f)) as u8);
            }
        }
    }
}

// RFC 1321 section 3: little-endian words, modulo-2^32 arithmetic, and
// the fixed sine-derived constants. This protocol specifically requires MD5.
fn md5(input: &[u8]) -> [u8; 16] {
    const ROTATION: [u32; 64] = [
        7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
        5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,
        4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
        6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21,
    ];
    const CONSTANT: [u32; 64] = [
        0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,
        0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,0x6b901122,0xfd987193,0xa679438e,0x49b40821,
        0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
        0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,
        0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,
        0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
        0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,
        0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391,
    ];
    let mut padded = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 { padded.push(0); }
    padded.extend_from_slice(&bits.to_le_bytes());
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in padded.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (word, bytes) in words.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for round in 0..64 {
            let (function, index) = match round {
                0..=15 => ((b & c) | (!b & d), round),
                16..=31 => ((d & b) | (!d & c), (5 * round + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * round + 5) % 16),
                _ => (c ^ (b | !d), (7 * round) % 16),
            };
            let next = b.wrapping_add(a.wrapping_add(function).wrapping_add(CONSTANT[round])
                .wrapping_add(words[index]).rotate_left(ROTATION[round]));
            a = d; d = c; c = b; b = next;
        }
        for (word, increment) in state.iter_mut().zip([a, b, c, d]) { *word = word.wrapping_add(increment); }
    }
    let mut digest = [0u8; 16];
    for (bytes, word) in digest.chunks_exact_mut(4).zip(state) { bytes.copy_from_slice(&word.to_le_bytes()); }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

    #[test]
    fn rfc1321_vectors_cover_padding_and_multiple_blocks() {
        for (input, expected) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            ("abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
            ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", "d174ab98d277d9f5a5611c2c9f419d9f"),
            ("12345678901234567890123456789012345678901234567890123456789012345678901234567890", "57edf4a22be3c955ac49da2e2107b67a"),
        ] { assert_eq!(hex(&md5(input.as_bytes())), expected); }
    }

    #[test]
    fn ascii_response_matches_independent_python_hashlib_vector() {
        let expected = b"7e6779cea6621467c6e5d4d985da43b7";
        assert_eq!(response("2.0;abc123", "CarLink").unwrap().as_slice(), expected);
        // Additional challenge fields do not become part of the nonce.
        assert_eq!(response("2.0;abc123;ignored", "CarLink").unwrap().as_slice(), expected);
    }

    #[test]
    fn jni_encoding_handles_nul_utf16_boundaries_and_surrogate_pairs() {
        let mut bytes = Vec::new();
        modified_utf8("\0\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{1f600}", &mut bytes);
        assert_eq!(bytes, vec![0xc0,0x80,0x7f,0xc2,0x80,0xdf,0xbf,0xe0,0xa0,0x80,
            0xef,0xbf,0xbf,0xed,0xa0,0xbd,0xed,0xb8,0x80]);
        // hashlib.md5(bytes.fromhex('4341524c49464541c08042eda0bdedb8806e'))
        assert_eq!(response("2.0;n", "A\0B😀").unwrap().as_slice(), b"f1ef118f4010531837703e9ec51ab0ff");
    }

    #[test]
    fn malformed_challenges_are_rejected_without_echoing_input() {
        for challenge in ["", "2.0", "1.0;secret-nonce", "2.0;", "2.0;;secret-nonce", " 2.0;x", "2.0x;x"] {
            let error = response(challenge, "private-identity").unwrap_err();
            assert!(!error.contains("secret-nonce"));
            assert!(!error.contains("private-identity"));
        }
        assert!(response(&format!("2.0;{}", "x".repeat(1020)), "id").is_ok());
        assert!(response(&format!("2.0;{}", "x".repeat(1021)), "id").is_err());
    }
}
