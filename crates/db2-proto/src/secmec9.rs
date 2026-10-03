//! DRDA SECMEC 0x0009 — Diffie-Hellman encrypted authentication.
//!
//! Implements the Diffie-Hellman key exchange and DES-CBC password/userid
//! encryption required by DB2's DRDA wire protocol for security mechanism 9.
//!
//! Uses OS randomness for private keys and big-endian Diffie-Hellman arithmetic.

use crate::codepage::utf8_to_ebcdic037;
use aes::Aes256;
use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use num_bigint::BigUint;
use sha1::{Digest, Sha1};

type Aes256CbcEnc = cbc::Encryptor<Aes256>;

/// Symmetric algorithm used after the DRDA Diffie-Hellman exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionAlgorithm {
    /// Classic DRDA DES-CBC with a 56-bit key.
    Des,
    /// DRDA AES-CBC with a 256-bit key.
    Aes,
}

// ---------------------------------------------------------------------------
// DH constants (256-bit, big-endian)
// ---------------------------------------------------------------------------

/// DH prime: 0xC62112D73EE613F0947AB31F0F6846A1BFF5B3A4CA0D60BC1E4C7A0D8C16B3E3
pub const DH_PRIME: [u8; 32] = [
    0xC6, 0x21, 0x12, 0xD7, 0x3E, 0xE6, 0x13, 0xF0, 0x94, 0x7A, 0xB3, 0x1F, 0x0F, 0x68, 0x46, 0xA1,
    0xBF, 0xF5, 0xB3, 0xA4, 0xCA, 0x0D, 0x60, 0xBC, 0x1E, 0x4C, 0x7A, 0x0D, 0x8C, 0x16, 0xB3, 0xE3,
];

/// DH base (generator): 0x4690FA1F7B9E1D4442C86C9114603FDECF071EDCEC5F626E21E256AED9EA34E4
pub const DH_BASE: [u8; 32] = [
    0x46, 0x90, 0xFA, 0x1F, 0x7B, 0x9E, 0x1D, 0x44, 0x42, 0xC8, 0x6C, 0x91, 0x14, 0x60, 0x3F, 0xDE,
    0xCF, 0x07, 0x1E, 0xDC, 0xEC, 0x5F, 0x62, 0x6E, 0x21, 0xE2, 0x56, 0xAE, 0xD9, 0xEA, 0x34, 0xE4,
];

/// DH prime used by IBM JCC for DRDA AES encrypted authentication.
pub const DH_PRIME_AES: [u8; 64] = [
    0xF2, 0x4F, 0x63, 0x15, 0x0E, 0xAA, 0x97, 0xCC, 0xE7, 0x8F, 0x57, 0x10, 0xC4, 0x5F, 0xAF, 0xBE,
    0xB7, 0x1C, 0xF6, 0xA8, 0x72, 0x4F, 0x63, 0x14, 0x0E, 0xAA, 0x97, 0xCC, 0xE7, 0x8F, 0x57, 0x10,
    0xC4, 0x5F, 0xAF, 0xBE, 0xB7, 0x1C, 0xF6, 0xA8, 0x72, 0x4F, 0x63, 0x13, 0x08, 0xE3, 0x2B, 0x26,
    0xEA, 0x15, 0x94, 0x88, 0x9C, 0xBB, 0xFC, 0x91, 0xF6, 0xDF, 0x75, 0x24, 0x35, 0x2E, 0xF9, 0x79,
];

/// DH base used by IBM JCC for DRDA AES encrypted authentication.
pub const DH_BASE_AES: [u8; 64] = [
    0xE8, 0xCE, 0x9E, 0x08, 0x44, 0xC6, 0x7A, 0x00, 0x9F, 0xB7, 0x84, 0x3C, 0xD9, 0x45, 0xA0, 0x58,
    0x93, 0x5D, 0xA5, 0x1B, 0x02, 0x8A, 0x49, 0xE5, 0xA9, 0x1F, 0x83, 0x1B, 0x78, 0x36, 0x44, 0x91,
    0xCD, 0x0E, 0x0A, 0x8F, 0x72, 0x34, 0x5D, 0xF8, 0x07, 0x69, 0x54, 0x99, 0x26, 0xFD, 0x16, 0xEC,
    0xD6, 0xF6, 0x85, 0x94, 0x81, 0x64, 0x7C, 0xA9, 0xEF, 0xB2, 0xBA, 0xAC, 0x7B, 0xC0, 0x9A, 0x92,
];

// ===========================================================================
// 256-bit big-endian unsigned integer arithmetic
// ===========================================================================

/// A 256-bit unsigned integer stored as 32 bytes in big-endian order.
#[cfg(test)]
#[derive(Clone, Copy)]
struct U256([u8; 32]);

#[cfg(test)]
impl U256 {
    const ZERO: Self = Self([0u8; 32]);
    const ONE: Self = Self([
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1,
    ]);

    fn from_bytes(b: &[u8]) -> Self {
        let mut arr = [0u8; 32];
        if b.len() >= 32 {
            arr.copy_from_slice(&b[b.len() - 32..]);
        } else {
            arr[32 - b.len()..].copy_from_slice(b);
        }
        Self(arr)
    }

    fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Return true if this value is zero.
    fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    /// Test whether bit `i` (0 = LSB) is set.
    fn bit(&self, i: usize) -> bool {
        if i >= 256 {
            return false;
        }
        let byte_idx = 31 - (i / 8);
        let bit_idx = i % 8;
        (self.0[byte_idx] >> bit_idx) & 1 == 1
    }

    /// Number of significant bits.
    fn bits(&self) -> usize {
        for i in 0..32 {
            if self.0[i] != 0 {
                let leading = self.0[i].leading_zeros() as usize;
                return (32 - i) * 8 - leading;
            }
        }
        0
    }

    /// Compare: -1, 0, 1.
    fn cmp(&self, other: &Self) -> i32 {
        for i in 0..32 {
            if self.0[i] < other.0[i] {
                return -1;
            }
            if self.0[i] > other.0[i] {
                return 1;
            }
        }
        0
    }

    /// self >= other
    fn gte(&self, other: &Self) -> bool {
        self.cmp(other) >= 0
    }

    /// Subtract other from self (self must be >= other).
    fn sub(&self, other: &Self) -> Self {
        let mut result = [0u8; 32];
        let mut borrow: u16 = 0;
        for i in (0..32).rev() {
            let a = self.0[i] as u16;
            let b = other.0[i] as u16 + borrow;
            if a >= b {
                result[i] = (a - b) as u8;
                borrow = 0;
            } else {
                result[i] = (256 + a - b) as u8;
                borrow = 1;
            }
        }
        Self(result)
    }

    /// Add two 256-bit numbers, returning (result, carry).
    fn add_with_carry(&self, other: &Self) -> (Self, bool) {
        let mut result = [0u8; 32];
        let mut carry: u16 = 0;
        for i in (0..32).rev() {
            let s = self.0[i] as u16 + other.0[i] as u16 + carry;
            result[i] = s as u8;
            carry = s >> 8;
        }
        (Self(result), carry != 0)
    }

    /// Shift left by 1 bit.
    fn shl1(&self) -> (Self, bool) {
        let mut result = [0u8; 32];
        let mut carry = 0u8;
        for i in (0..32).rev() {
            let new_carry = self.0[i] >> 7;
            result[i] = (self.0[i] << 1) | carry;
            carry = new_carry;
        }
        (Self(result), carry != 0)
    }

    /// Shift right by 1 bit.
    fn shr1(&self) -> Self {
        let mut result = [0u8; 32];
        let mut carry = 0u8;
        for (i, byte) in self.0.iter().enumerate() {
            let new_carry = byte << 7;
            result[i] = (byte >> 1) | carry;
            carry = new_carry;
        }
        Self(result)
    }

    /// Modular reduction: self mod m.
    /// Uses shift-and-subtract algorithm.
    fn modulo(&self, m: &Self) -> Self {
        if m.is_zero() {
            return Self::ZERO;
        }
        if self.cmp(m) < 0 {
            return *self;
        }

        // Find how many bits to shift m left so its MSB aligns with self's MSB.
        let self_bits = self.bits();
        let m_bits = m.bits();
        if self_bits == 0 {
            return Self::ZERO;
        }
        if m_bits == 0 {
            return Self::ZERO;
        }

        let mut remainder = *self;
        let shift = self_bits - m_bits;

        // Shift m left by `shift` bits.
        let mut shifted_m = *m;
        for _ in 0..shift {
            let (s, _overflow) = shifted_m.shl1();
            shifted_m = s;
        }

        for _ in 0..=shift {
            if remainder.gte(&shifted_m) {
                remainder = remainder.sub(&shifted_m);
            }
            shifted_m = shifted_m.shr1();
        }

        remainder
    }

    /// Modular multiplication: (self * other) mod m.
    /// Uses the shift-and-add approach on a 512-bit intermediate.
    fn mulmod(&self, other: &Self, m: &Self) -> Self {
        // We accumulate in a U256, reducing after each addition to keep within range.
        let mut result = Self::ZERO;
        let mut a = self.modulo(m);
        let b_bits = other.bits();

        for i in 0..b_bits {
            if other.bit(i) {
                let (sum, carry) = result.add_with_carry(&a);
                result = if carry || sum.gte(m) {
                    // Need to reduce: since result < m and a < m, sum < 2m,
                    // so a single subtract suffices if no carry. With carry, sum = 2^256 + val,
                    // but since m < 2^256, we need to handle this.
                    if carry {
                        // sum = 2^256 + val, subtract m: result = (2^256 + val) - m
                        // Since m < 2^256, 2^256 - m is positive. We compute val - m + 2^256 mod m.
                        // val is the truncated sum. 2^256 mod m = (2^256 - m) if m <= 2^256.
                        // Simpler: (2^256 + val) mod m. Since val < 2^256 and m < 2^256:
                        // 2^256 mod m = 2^256 - m (since 2^256 = 1*m + (2^256 - m) if m < 2^256)
                        // Wait, 2^256 / m could be > 1. Let's just subtract m iteratively.
                        let two256_mod_m = {
                            // 2^256 = (2^256 - prime). For our prime this is small.
                            // Actually let's just compute it properly.
                            // 2^256 mod m: we know 2^256 won't fit in U256, but we can compute
                            // by noting 2^256 = m * q + r. We do: -(m) mod 2^256 = 2^256 - m.
                            // Then 2^256 mod m = (2^256 - m) mod m if 2^256 - m < m then it's the answer,
                            // otherwise keep subtracting.
                            let neg_m = Self::ZERO.sub(m); // wrapping: 2^256 - m
                            neg_m.modulo(m)
                        };
                        let (s2, c2) = sum.add_with_carry(&two256_mod_m);
                        if c2 || s2.gte(m) {
                            s2.sub(m)
                        } else {
                            s2
                        }
                    } else {
                        sum.sub(m)
                    }
                } else {
                    sum
                };
            }
            // Double a mod m
            let (dbl, dbl_carry) = a.add_with_carry(&a);
            a = if dbl_carry || dbl.gte(m) {
                if dbl_carry {
                    let neg_m = Self::ZERO.sub(m);
                    let neg_m_mod = neg_m.modulo(m);
                    let (s2, c2) = dbl.add_with_carry(&neg_m_mod);
                    if c2 || s2.gte(m) {
                        s2.sub(m)
                    } else {
                        s2
                    }
                } else {
                    dbl.sub(m)
                }
            } else {
                dbl
            };
        }

        result
    }

    /// Modular exponentiation: self^exp mod m using repeated squaring.
    fn powmod(&self, exp: &Self, m: &Self) -> Self {
        if m.is_zero() {
            return Self::ZERO;
        }
        let mut result = Self::ONE.modulo(m);
        let mut base = self.modulo(m);
        let exp_bits = exp.bits();

        for i in 0..exp_bits {
            if exp.bit(i) {
                result = result.mulmod(&base, m);
            }
            base = base.mulmod(&base, m);
        }

        result
    }
}

// ===========================================================================
// Public DH API
// ===========================================================================

/// Generate a random 32-byte private key that is less than the DH prime.
///
/// Uses a simple xorshift-based PRNG seeded from stack/heap entropy.
/// This is adequate for the DB2 auth handshake but is NOT cryptographically
/// secure in the formal sense. For production use, consider reading from
/// `/dev/urandom` or using a proper CSPRNG.
pub fn generate_private_key() -> Vec<u8> {
    generate_private_key_with_algorithm(EncryptionAlgorithm::Des)
}

/// Generate a private key for the selected DRDA encrypted credential algorithm.
pub fn generate_private_key_with_algorithm(algorithm: EncryptionAlgorithm) -> Vec<u8> {
    try_generate_private_key_with_algorithm(algorithm).expect("OS random source unavailable")
}

/// Fallible key generation used by authentication, so OS failures become connection errors.
pub fn try_generate_private_key_with_algorithm(
    algorithm: EncryptionAlgorithm,
) -> crate::Result<Vec<u8>> {
    let (prime_bytes, len) = dh_parameters(algorithm);
    let prime = BigUint::from_bytes_be(prime_bytes);
    loop {
        let mut bytes = vec![0u8; len];
        getrandom::getrandom(&mut bytes).map_err(|error| {
            crate::ProtoError::Other(format!("Cannot generate DRDA private key: {error}"))
        })?;
        let private = BigUint::from_bytes_be(&bytes);
        if private != BigUint::from(0u8) && private < prime {
            return Ok(bytes);
        }
    }
}

/// Calculate the DH public key: base^private mod prime.
///
/// Returns a 32-byte big-endian result.
pub fn calculate_public_key(private_key: &[u8]) -> Vec<u8> {
    calculate_public_key_with_algorithm(private_key, EncryptionAlgorithm::Des)
}

/// Calculate the DH public key for the selected encrypted credential algorithm.
pub fn calculate_public_key_with_algorithm(
    private_key: &[u8],
    algorithm: EncryptionAlgorithm,
) -> Vec<u8> {
    let (prime, len) = dh_parameters(algorithm);
    let base = dh_base(algorithm);
    let result = BigUint::from_bytes_be(base).modpow(
        &BigUint::from_bytes_be(private_key),
        &BigUint::from_bytes_be(prime),
    );
    fixed_len_bytes(&result, len)
}

/// Calculate the shared session key: server_public^client_private mod prime.
///
/// Returns a 32-byte big-endian result.
pub fn calculate_session_key(server_public: &[u8], client_private: &[u8]) -> Vec<u8> {
    calculate_session_key_with_algorithm(server_public, client_private, EncryptionAlgorithm::Des)
}

/// Calculate the raw shared session secret for the selected encrypted credential algorithm.
pub fn calculate_session_key_with_algorithm(
    server_public: &[u8],
    client_private: &[u8],
    algorithm: EncryptionAlgorithm,
) -> Vec<u8> {
    let (prime, len) = dh_parameters(algorithm);
    let result = BigUint::from_bytes_be(server_public).modpow(
        &BigUint::from_bytes_be(client_private),
        &BigUint::from_bytes_be(prime),
    );
    fixed_len_bytes(&result, len)
}

fn dh_parameters(algorithm: EncryptionAlgorithm) -> (&'static [u8], usize) {
    match algorithm {
        EncryptionAlgorithm::Des => (&DH_PRIME, DH_PRIME.len()),
        EncryptionAlgorithm::Aes => (&DH_PRIME_AES, DH_PRIME_AES.len()),
    }
}

fn dh_base(algorithm: EncryptionAlgorithm) -> &'static [u8] {
    match algorithm {
        EncryptionAlgorithm::Des => &DH_BASE,
        EncryptionAlgorithm::Aes => &DH_BASE_AES,
    }
}

fn fixed_len_bytes(value: &BigUint, len: usize) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    if bytes.len() == len + 1 && bytes[0] == 0 {
        bytes[1..].to_vec()
    } else if bytes.len() < len {
        let mut out = vec![0u8; len];
        out[len - bytes.len()..].copy_from_slice(&bytes);
        out
    } else {
        bytes
    }
}

// ===========================================================================
// DES implementation (single DES, CBC mode, PKCS5 padding)
// ===========================================================================

/// DES initial permutation table (IP).
#[rustfmt::skip]
const IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10,  2,
    60, 52, 44, 36, 28, 20, 12,  4,
    62, 54, 46, 38, 30, 22, 14,  6,
    64, 56, 48, 40, 32, 24, 16,  8,
    57, 49, 41, 33, 25, 17,  9,  1,
    59, 51, 43, 35, 27, 19, 11,  3,
    61, 53, 45, 37, 29, 21, 13,  5,
    63, 55, 47, 39, 31, 23, 15,  7,
];

/// DES final permutation table (IP^-1).
#[rustfmt::skip]
const FP: [u8; 64] = [
    40,  8, 48, 16, 56, 24, 64, 32,
    39,  7, 47, 15, 55, 23, 63, 31,
    38,  6, 46, 14, 54, 22, 62, 30,
    37,  5, 45, 13, 53, 21, 61, 29,
    36,  4, 44, 12, 52, 20, 60, 28,
    35,  3, 43, 11, 51, 19, 59, 27,
    34,  2, 42, 10, 50, 18, 58, 26,
    33,  1, 41,  9, 49, 17, 57, 25,
];

/// DES expansion permutation (E).
#[rustfmt::skip]
const E: [u8; 48] = [
    32,  1,  2,  3,  4,  5,
     4,  5,  6,  7,  8,  9,
     8,  9, 10, 11, 12, 13,
    12, 13, 14, 15, 16, 17,
    16, 17, 18, 19, 20, 21,
    20, 21, 22, 23, 24, 25,
    24, 25, 26, 27, 28, 29,
    28, 29, 30, 31, 32,  1,
];

/// DES permutation (P) after S-box substitution.
#[rustfmt::skip]
const P: [u8; 32] = [
    16,  7, 20, 21, 29, 12, 28, 17,
     1, 15, 23, 26,  5, 18, 31, 10,
     2,  8, 24, 14, 32, 27,  3,  9,
    19, 13, 30,  6, 22, 11,  4, 25,
];

/// DES permuted choice 1 (PC-1), selecting 56 bits from 64-bit key.
#[rustfmt::skip]
const PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17,  9,
     1, 58, 50, 42, 34, 26, 18,
    10,  2, 59, 51, 43, 35, 27,
    19, 11,  3, 60, 52, 44, 36,
    63, 55, 47, 39, 31, 23, 15,
     7, 62, 54, 46, 38, 30, 22,
    14,  6, 61, 53, 45, 37, 29,
    21, 13,  5, 28, 20, 12,  4,
];

/// DES permuted choice 2 (PC-2), selecting 48 bits from 56 bits.
#[rustfmt::skip]
const PC2: [u8; 48] = [
    14, 17, 11, 24,  1,  5,
     3, 28, 15,  6, 21, 10,
    23, 19, 12,  4, 26,  8,
    16,  7, 27, 20, 13,  2,
    41, 52, 31, 37, 47, 55,
    30, 40, 51, 45, 33, 48,
    44, 49, 39, 56, 34, 53,
    46, 42, 50, 36, 29, 32,
];

/// DES key schedule rotation amounts.
const KEY_SHIFTS: [u8; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

/// DES S-boxes (8 boxes, each 4 rows x 16 columns).
#[rustfmt::skip]
const SBOXES: [[[u8; 16]; 4]; 8] = [
    // S1
    [
        [14,4,13,1,2,15,11,8,3,10,6,12,5,9,0,7],
        [0,15,7,4,14,2,13,1,10,6,12,11,9,5,3,8],
        [4,1,14,8,13,6,2,11,15,12,9,7,3,10,5,0],
        [15,12,8,2,4,9,1,7,5,11,3,14,10,0,6,13],
    ],
    // S2
    [
        [15,1,8,14,6,11,3,4,9,7,2,13,12,0,5,10],
        [3,13,4,7,15,2,8,14,12,0,1,10,6,9,11,5],
        [0,14,7,11,10,4,13,1,5,8,12,6,9,3,2,15],
        [13,8,10,1,3,15,4,2,11,6,7,12,0,5,14,9],
    ],
    // S3
    [
        [10,0,9,14,6,3,15,5,1,13,12,7,11,4,2,8],
        [13,7,0,9,3,4,6,10,2,8,5,14,12,11,15,1],
        [13,6,4,9,8,15,3,0,11,1,2,12,5,10,14,7],
        [1,10,13,0,6,9,8,7,4,15,14,3,11,5,2,12],
    ],
    // S4
    [
        [7,13,14,3,0,6,9,10,1,2,8,5,11,12,4,15],
        [13,8,11,5,6,15,0,3,4,7,2,12,1,10,14,9],
        [10,6,9,0,12,11,7,13,15,1,3,14,5,2,8,4],
        [3,15,0,6,10,1,13,8,9,4,5,11,12,7,2,14],
    ],
    // S5
    [
        [2,12,4,1,7,10,11,6,8,5,3,15,13,0,14,9],
        [14,11,2,12,4,7,13,1,5,0,15,10,3,9,8,6],
        [4,2,1,11,10,13,7,8,15,9,12,5,6,3,0,14],
        [11,8,12,7,1,14,2,13,6,15,0,9,10,4,5,3],
    ],
    // S6
    [
        [12,1,10,15,9,2,6,8,0,13,3,4,14,7,5,11],
        [10,15,4,2,7,12,9,5,6,1,13,14,0,11,3,8],
        [9,14,15,5,2,8,12,3,7,0,4,10,1,13,11,6],
        [4,3,2,12,9,5,15,10,11,14,1,7,6,0,8,13],
    ],
    // S7
    [
        [4,11,2,14,15,0,8,13,3,12,9,7,5,10,6,1],
        [13,0,11,7,4,9,1,10,14,3,5,12,2,15,8,6],
        [1,4,11,13,12,3,7,14,10,15,6,8,0,5,9,2],
        [6,11,13,8,1,4,10,7,9,5,0,15,14,2,3,12],
    ],
    // S8
    [
        [13,2,8,4,6,15,11,1,10,9,3,14,5,0,12,7],
        [1,15,13,8,10,3,7,4,12,5,6,11,0,14,9,2],
        [7,11,4,1,9,12,14,2,0,6,10,13,15,3,5,8],
        [2,1,14,7,4,10,8,13,15,12,9,0,3,5,6,11],
    ],
];

// ---------------------------------------------------------------------------
// DES helper: u64-based bit manipulation
// ---------------------------------------------------------------------------

/// Convert 8 bytes (big-endian) to u64.
fn bytes_to_u64(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for &byte in b.iter().take(8) {
        v = (v << 8) | byte as u64;
    }
    v
}

/// Convert u64 to 8 bytes (big-endian).
fn u64_to_bytes(v: u64) -> [u8; 8] {
    [
        (v >> 56) as u8,
        (v >> 48) as u8,
        (v >> 40) as u8,
        (v >> 32) as u8,
        (v >> 24) as u8,
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ]
}

/// Apply a permutation to a u64 value.
/// `table` is 1-indexed (DES convention). Bit 1 = MSB (bit 63 of u64).
/// Output bit at position `i` (0-indexed from MSB) comes from input bit `table[i]`.
fn permute_u64(input: u64, table: &[u8]) -> u64 {
    let mut output = 0u64;
    for (i, &pos) in table.iter().enumerate() {
        // Input bit `pos` (1-indexed from MSB) = bit (64 - pos) of u64
        let bit = (input >> (64 - pos as u32)) & 1;
        // Output bit `i` (0-indexed from MSB) = bit (63 - i) of u64
        // But we only use the top `table.len()` bits of output.
        output |= bit << (table.len() - 1 - i);
    }
    output
}

/// Apply a permutation where the output has fewer bits than 64.
/// Returns the result left-aligned in the u64 (MSB = bit 0 of output).
fn permute_bits(input: u64, input_bits: u32, table: &[u8]) -> u64 {
    let mut output = 0u64;
    for (i, &pos) in table.iter().enumerate() {
        let bit = (input >> (input_bits - pos as u32)) & 1;
        output |= bit << (table.len() - 1 - i);
    }
    output
}

/// Left-rotate a 28-bit value by n positions.
fn rotate_left_28(val: u32, n: u32) -> u32 {
    ((val << n) | (val >> (28 - n))) & 0x0FFF_FFFF
}

/// Generate 16 round subkeys from the 8-byte DES key.
fn des_key_schedule(key: &[u8; 8]) -> [u64; 16] {
    let key64 = bytes_to_u64(key);

    // Apply PC-1 to get 56-bit key (stored in lower 56 bits).
    let pc1_out = permute_bits(key64, 64, &PC1);

    // Split into C (upper 28 bits) and D (lower 28 bits).
    let mut c = (pc1_out >> 28) as u32 & 0x0FFF_FFFF;
    let mut d = pc1_out as u32 & 0x0FFF_FFFF;

    let mut subkeys = [0u64; 16];

    for round in 0..16 {
        let shift = KEY_SHIFTS[round] as u32;
        c = rotate_left_28(c, shift);
        d = rotate_left_28(d, shift);

        // Combine C and D into 56 bits.
        let cd: u64 = ((c as u64) << 28) | (d as u64);

        // Apply PC-2 to get 48-bit subkey.
        subkeys[round] = permute_bits(cd, 56, &PC2);
    }

    subkeys
}

/// The DES f-function: expand R, XOR with subkey, S-box, permute.
fn des_f(r: u32, subkey: u64) -> u32 {
    // Expand R from 32 to 48 bits using E table.
    let mut expanded = 0u64;
    for (i, &pos) in E.iter().enumerate() {
        let bit = ((r >> (32 - pos as u32)) & 1) as u64;
        expanded |= bit << (47 - i as u32);
    }

    // XOR with subkey (both are 48 bits in lower bits).
    let xored = expanded ^ subkey;

    // S-box substitution: 48 bits -> 32 bits.
    let mut sbox_out = 0u32;
    for i in 0..8u32 {
        // Extract 6 bits for S-box i (from MSB of 48-bit value).
        let shift = 42 - i * 6;
        let six_bits = ((xored >> shift) & 0x3F) as u8;

        // Row = outer bits (bit5, bit0), Column = inner bits (bit4..bit1).
        let row = ((six_bits >> 4) & 0x02) | (six_bits & 0x01);
        let col = (six_bits >> 1) & 0x0F;

        let sval = SBOXES[i as usize][row as usize][col as usize] as u32;
        sbox_out |= sval << (28 - i * 4);
    }

    // Apply permutation P.
    let mut result = 0u32;
    for (i, &pos) in P.iter().enumerate() {
        let bit = (sbox_out >> (32 - pos as u32)) & 1;
        result |= bit << (31 - i as u32);
    }

    result
}

/// Encrypt a single 8-byte block with DES.
fn des_encrypt_block(block: &[u8; 8], subkeys: &[u64; 16]) -> [u8; 8] {
    let block64 = bytes_to_u64(block);

    // Initial permutation.
    let ip_out = permute_u64(block64, &IP);
    // ip_out has 64 meaningful bits in the lower 64 bits of u64.

    let mut l = (ip_out >> 32) as u32;
    let mut r = ip_out as u32;

    // 16 Feistel rounds.
    for subkey in subkeys {
        let f_out = des_f(r, *subkey);
        let new_r = l ^ f_out;
        l = r;
        r = new_r;
    }

    // Combine as R||L (the final swap).
    let pre_fp = ((r as u64) << 32) | (l as u64);

    // Final permutation.
    let result = permute_u64(pre_fp, &FP);

    u64_to_bytes(result)
}

/// Encrypt data using DES-CBC with PKCS5 padding.
fn des_cbc_encrypt(key: &[u8; 8], iv: &[u8; 8], plaintext: &[u8]) -> Vec<u8> {
    let subkeys = des_key_schedule(key);

    // PKCS5 padding
    let pad_len = 8 - (plaintext.len() % 8);
    let mut padded = plaintext.to_vec();
    for _ in 0..pad_len {
        padded.push(pad_len as u8);
    }

    let mut ciphertext = Vec::with_capacity(padded.len());
    let mut prev_block = *iv;

    for chunk in padded.chunks(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(chunk);

        // XOR with previous ciphertext block (CBC)
        for i in 0..8 {
            block[i] ^= prev_block[i];
        }

        let encrypted = des_encrypt_block(&block, &subkeys);
        ciphertext.extend_from_slice(&encrypted);
        prev_block = encrypted;
    }

    ciphertext
}

// ===========================================================================
// Password / UserID encryption for SECMEC 9
// ===========================================================================

/// Encrypt a password for DRDA SECMEC 0x0009 authentication.
///
/// - DES key  = `session_key[12..20]`
/// - IV       = `server_sectkn[12..20]`
/// - Data     = password converted to EBCDIC 037, then PKCS5-padded
pub fn encrypt_password(session_key: &[u8], server_sectkn: &[u8], password: &str) -> Vec<u8> {
    let ebcdic_password = utf8_to_ebcdic037(password);
    encrypt_password_bytes(session_key, server_sectkn, &ebcdic_password)
}

/// Encrypt pre-encoded password bytes for DRDA SECMEC 0x0009 authentication.
pub fn encrypt_password_bytes(
    session_key: &[u8],
    server_sectkn: &[u8],
    password: &[u8],
) -> Vec<u8> {
    encrypt_password_bytes_with_algorithm(
        session_key,
        server_sectkn,
        password,
        EncryptionAlgorithm::Des,
    )
}

/// Encrypt pre-encoded password bytes with the selected DRDA encryption algorithm.
pub fn encrypt_password_bytes_with_algorithm(
    session_key: &[u8],
    server_sectkn: &[u8],
    password: &[u8],
    algorithm: EncryptionAlgorithm,
) -> Vec<u8> {
    match algorithm {
        EncryptionAlgorithm::Des => {
            let mut iv = [0u8; 8];
            iv.copy_from_slice(&server_sectkn[12..20]);
            encrypt_bytes_with_iv(session_key, &iv, password)
        }
        EncryptionAlgorithm::Aes => {
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&server_sectkn[24..40]);
            aes_cbc_encrypt(session_key, &iv, password)
        }
    }
}

/// Encrypt pre-encoded user ID bytes with the selected DRDA encryption algorithm.
pub fn encrypt_userid_bytes_with_algorithm(
    session_key: &[u8],
    server_sectkn: &[u8],
    userid: &[u8],
    algorithm: EncryptionAlgorithm,
) -> Vec<u8> {
    match algorithm {
        EncryptionAlgorithm::Des => {
            let mut iv = [0u8; 8];
            iv.copy_from_slice(&server_sectkn[12..20]);
            encrypt_bytes_with_iv(session_key, &iv, userid)
        }
        EncryptionAlgorithm::Aes => {
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&server_sectkn[24..40]);
            aes_cbc_encrypt(session_key, &iv, userid)
        }
    }
}

/// Encrypt pre-encoded password bytes for DRDA SECMEC 0x0007 authentication
/// with the selected DRDA encryption algorithm.
///
/// DES uses the user ID bytes as the IV. AES follows IBM JCC and uses bytes
/// 24..40 from the server security token as the IV.
pub fn encrypt_password_with_userid_iv_bytes_with_algorithm(
    session_key: &[u8],
    server_sectkn: &[u8],
    userid: &[u8],
    password: &[u8],
    algorithm: EncryptionAlgorithm,
) -> Vec<u8> {
    match algorithm {
        EncryptionAlgorithm::Des => {
            encrypt_password_with_userid_iv_bytes(session_key, userid, password)
        }
        EncryptionAlgorithm::Aes => {
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&server_sectkn[24..40]);
            aes_cbc_encrypt(session_key, &iv, password)
        }
    }
}

fn aes_cbc_encrypt(session_key: &[u8], iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let aes_key = derive_aes_key_from_dh_secret(session_key);

    Aes256CbcEnc::new(&aes_key.into(), iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plaintext)
}

fn derive_aes_key_from_dh_secret(session_key: &[u8]) -> [u8; 32] {
    let mut normalized = [0u8; 64];
    if session_key.len() == 65 && session_key[0] == 0 {
        normalized.copy_from_slice(&session_key[1..65]);
    } else if session_key.len() < 64 {
        normalized[64 - session_key.len()..].copy_from_slice(session_key);
    } else {
        normalized.copy_from_slice(&session_key[..64]);
    }

    let mut digest = Sha1::new();
    digest.update(&normalized[..32]);
    let first = digest.finalize_reset();

    digest.update(&normalized[32..64]);
    let second = digest.finalize();

    let mut aes_key = [0u8; 32];
    aes_key[..12].copy_from_slice(&first[..12]);
    for i in 0..8 {
        aes_key[12 + i] = first[12 + i] ^ second[i];
    }
    aes_key[20..32].copy_from_slice(&second[8..20]);
    aes_key
}

/// Encrypt a user ID for DRDA SECMEC 0x0009 authentication.
///
/// Same algorithm as [`encrypt_password`] but applied to the user ID.
pub fn encrypt_userid(session_key: &[u8], server_sectkn: &[u8], userid: &str) -> Vec<u8> {
    let ebcdic_userid = utf8_to_ebcdic037(userid);
    encrypt_userid_bytes(session_key, server_sectkn, &ebcdic_userid)
}

/// Encrypt pre-encoded user ID bytes for DRDA SECMEC 0x0009 authentication.
pub fn encrypt_userid_bytes(session_key: &[u8], server_sectkn: &[u8], userid: &[u8]) -> Vec<u8> {
    let mut iv = [0u8; 8];
    iv.copy_from_slice(&server_sectkn[12..20]);

    encrypt_bytes_with_iv(session_key, &iv, userid)
}

/// Encrypt a password for DRDA SECMEC 0x0007 authentication.
///
/// SECMEC 7 sends the user ID in clear text and uses the EBCDIC user ID,
/// zero-padded or truncated to 8 bytes, as the DES-CBC IV.
pub fn encrypt_password_with_userid_iv(
    session_key: &[u8],
    userid: &str,
    password: &str,
) -> Vec<u8> {
    let userid_bytes = utf8_to_ebcdic037(userid);
    let password_bytes = utf8_to_ebcdic037(password);

    encrypt_password_with_userid_iv_bytes(session_key, &userid_bytes, &password_bytes)
}

/// Encrypt pre-encoded password bytes for DRDA SECMEC 0x0007 authentication.
///
/// The pre-encoded user ID bytes are zero-padded or truncated to 8 bytes and
/// used as the DES-CBC IV.
pub fn encrypt_password_with_userid_iv_bytes(
    session_key: &[u8],
    userid: &[u8],
    password: &[u8],
) -> Vec<u8> {
    let mut iv = [0u8; 8];
    let copy_len = userid.len().min(8);
    iv[..copy_len].copy_from_slice(&userid[..copy_len]);

    encrypt_bytes_with_iv(session_key, &iv, password)
}

fn encrypt_bytes_with_iv(session_key: &[u8], iv: &[u8; 8], plaintext: &[u8]) -> Vec<u8> {
    let mut des_key = [0u8; 8];
    des_key.copy_from_slice(&session_key[12..20]);

    des_cbc_encrypt(&des_key, iv, plaintext)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u256_basic_ops() {
        let zero = U256::ZERO;
        let one = U256::ONE;
        assert!(zero.is_zero());
        assert!(!one.is_zero());
        assert_eq!(zero.cmp(&one), -1);
        assert_eq!(one.cmp(&zero), 1);
        assert_eq!(one.cmp(&one), 0);
    }

    #[test]
    fn test_u256_sub() {
        let a = U256::from_bytes(&[0, 0, 0, 10]);
        let b = U256::from_bytes(&[0, 0, 0, 3]);
        let c = a.sub(&b);
        assert_eq!(c.to_bytes()[31], 7);
    }

    #[test]
    fn test_u256_modulo() {
        let a = U256::from_bytes(&[0, 0, 0, 17]);
        let m = U256::from_bytes(&[0, 0, 0, 5]);
        let r = a.modulo(&m);
        assert_eq!(r.to_bytes()[31], 2);
    }

    #[test]
    fn test_u256_mulmod() {
        // 7 * 8 mod 13 = 56 mod 13 = 4
        let a = U256::from_bytes(&[7]);
        let b = U256::from_bytes(&[8]);
        let m = U256::from_bytes(&[13]);
        let r = a.mulmod(&b, &m);
        assert_eq!(r.to_bytes()[31], 4);
    }

    #[test]
    fn test_u256_powmod_small() {
        // 3^13 mod 7 = 1594323 mod 7 = 3
        let base = U256::from_bytes(&[3]);
        let exp = U256::from_bytes(&[13]);
        let m = U256::from_bytes(&[7]);
        let r = base.powmod(&exp, &m);
        assert_eq!(r.to_bytes()[31], 3);
    }

    #[test]
    fn test_u256_powmod_medium() {
        // 2^10 mod 1000 = 1024 mod 1000 = 24
        let base = U256::from_bytes(&[2]);
        let exp = U256::from_bytes(&[10]);
        let m = U256::from_bytes(&[0x03, 0xE8]); // 1000
        let r = base.powmod(&exp, &m);
        let val = (r.to_bytes()[30] as u16) << 8 | r.to_bytes()[31] as u16;
        assert_eq!(val, 24);
    }

    #[test]
    fn test_dh_key_exchange_consistency() {
        // Generate two private keys, compute public keys, verify shared secret matches.
        let priv_a = generate_private_key();
        let priv_b = generate_private_key();
        let pub_a = calculate_public_key(&priv_a);
        let pub_b = calculate_public_key(&priv_b);

        // shared_a = pub_b^priv_a mod prime
        let shared_a = calculate_session_key(&pub_b, &priv_a);
        // shared_b = pub_a^priv_b mod prime
        let shared_b = calculate_session_key(&pub_a, &priv_b);

        assert_eq!(shared_a, shared_b, "DH shared secrets must match");
    }

    #[test]
    fn test_dh_public_key_known_vectors() {
        let cases = [
            (
                "0000000000000000000000000000000000000000000000000000000000000001",
                "4690fa1f7b9e1d4442c86c9114603fdecf071edcec5f626e21e256aed9ea34e4",
            ),
            (
                "0000000000000000000000000000000000000000000000000000000000000002",
                "2be2e3ebeed26cdf9f0543b8b471390a3ec4a4d5fe3265e3333ce57365dde634",
            ),
            (
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "144eabbd93f2b7a1b961cd058e390dd9883c9bc4e4b669685603e79bf0fe3a78",
            ),
        ];

        for (private_hex, public_hex) in cases {
            let private_key = hex_to_bytes(private_hex);
            let expected_public = hex_to_bytes(public_hex);
            assert_eq!(calculate_public_key(&private_key), expected_public);
        }
    }

    #[test]
    fn test_aes_dh_public_key_known_vector() {
        let private_key = {
            let mut key = vec![0u8; 64];
            key[63] = 1;
            key
        };

        assert_eq!(
            calculate_public_key_with_algorithm(&private_key, EncryptionAlgorithm::Aes),
            DH_BASE_AES.to_vec()
        );
    }

    #[test]
    fn test_aes_dh_key_exchange_consistency() {
        let priv_a = generate_private_key_with_algorithm(EncryptionAlgorithm::Aes);
        let priv_b = generate_private_key_with_algorithm(EncryptionAlgorithm::Aes);
        let pub_a = calculate_public_key_with_algorithm(&priv_a, EncryptionAlgorithm::Aes);
        let pub_b = calculate_public_key_with_algorithm(&priv_b, EncryptionAlgorithm::Aes);

        let shared_a =
            calculate_session_key_with_algorithm(&pub_b, &priv_a, EncryptionAlgorithm::Aes);
        let shared_b =
            calculate_session_key_with_algorithm(&pub_a, &priv_b, EncryptionAlgorithm::Aes);

        assert_eq!(pub_a.len(), 64);
        assert_eq!(pub_b.len(), 64);
        assert_eq!(shared_a.len(), 64);
        assert_eq!(shared_a, shared_b);
    }

    #[test]
    fn test_des_known_vectors() {
        // Test vector verified against OpenSSL:
        // key = 0133457799BBCDFF, plaintext = 0123456789ABCDEF
        let key1: [u8; 8] = [0x01, 0x33, 0x45, 0x77, 0x99, 0xBB, 0xCD, 0xFF];
        let pt1: [u8; 8] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let expected1: [u8; 8] = [0x1E, 0xD2, 0xCD, 0x64, 0x84, 0x90, 0x78, 0xB9];
        let subkeys1 = des_key_schedule(&key1);
        let result1 = des_encrypt_block(&pt1, &subkeys1);
        assert_eq!(result1, expected1, "DES test 1 failed");

        // NIST KAT: key = 0101010101010101 (effective zero key),
        // plaintext = 8000000000000000, ciphertext = 95F8A5E5DD31D900
        let key2: [u8; 8] = [0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01];
        let pt2: [u8; 8] = [0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let expected2: [u8; 8] = [0x95, 0xF8, 0xA5, 0xE5, 0xDD, 0x31, 0xD9, 0x00];
        let subkeys2 = des_key_schedule(&key2);
        let result2 = des_encrypt_block(&pt2, &subkeys2);
        assert_eq!(result2, expected2, "DES test 2 (NIST KAT) failed");

        // FIPS 46-3 Appendix B example:
        // key = 133457799BBCDFF1, PT = 0123456789ABCDEF, CT = 85E813540F0AB405
        let key3: [u8; 8] = [0x13, 0x34, 0x57, 0x79, 0x9B, 0xBC, 0xDF, 0xF1];
        let pt3: [u8; 8] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        let expected3: [u8; 8] = [0x85, 0xE8, 0x13, 0x54, 0x0F, 0x0A, 0xB4, 0x05];
        let subkeys3 = des_key_schedule(&key3);
        let result3 = des_encrypt_block(&pt3, &subkeys3);
        assert_eq!(result3, expected3, "DES test 3 (FIPS appendix B) failed");
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        assert_eq!(hex.len() % 2, 0);
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn test_des_cbc_pkcs5() {
        // CBC vectors covering S8 inputs missed by the original block vectors.
        assert_eq!(
            des_cbc_encrypt(&[0; 8], &[0; 8], &hex_to_bytes("00000000000000b1")),
            hex_to_bytes("0ad0f2fba4ac0a2316adc4e166fc710f")
        );
        assert_eq!(
            des_cbc_encrypt(
                &hex_to_bytes("5851f42d4c957f2d").try_into().unwrap(),
                &hex_to_bytes("14057b7ef767814f").try_into().unwrap(),
                &hex_to_bytes("00000000000000b2")
            ),
            hex_to_bytes("97ef587910fc729d5bf9e98f62ff1bc6")
        );
        // Verify PKCS5 padding is applied correctly.
        let key = [0u8; 8];
        let iv = [0u8; 8];

        // 3 bytes of plaintext -> 8 bytes padded (5 bytes of 0x05 padding)
        let ct = des_cbc_encrypt(&key, &iv, &[1, 2, 3]);
        assert_eq!(ct.len(), 8);

        // 8 bytes of plaintext -> 16 bytes (full block of 0x08 padding)
        let ct2 = des_cbc_encrypt(&key, &iv, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(ct2.len(), 16);
    }

    #[test]
    fn test_encrypt_password_does_not_panic() {
        // Verify encrypt_password runs without panicking for typical inputs.
        let session_key = vec![0u8; 32];
        let server_sectkn = vec![0u8; 32];
        let ct = encrypt_password(&session_key, &server_sectkn, "mypassword");
        // EBCDIC "mypassword" = 10 bytes, padded to 16
        assert_eq!(ct.len(), 16);
    }

    #[test]
    fn test_encrypt_userid_does_not_panic() {
        let session_key = vec![0u8; 32];
        let server_sectkn = vec![0u8; 32];
        let ct = encrypt_userid(&session_key, &server_sectkn, "db2admin");
        // "db2admin" = 8 EBCDIC bytes, PKCS5 adds a full block of padding -> 16 bytes
        assert_eq!(ct.len(), 16);
    }

    #[test]
    fn test_aes_encryption_uses_16_byte_blocks() {
        let session_key = vec![0x11; 64];
        let server_sectkn = vec![0x22; 64];
        let ct = encrypt_password_bytes_with_algorithm(
            &session_key,
            &server_sectkn,
            b"mypassword",
            EncryptionAlgorithm::Aes,
        );
        assert_eq!(ct.len(), 16);

        let ct2 = encrypt_password_with_userid_iv_bytes_with_algorithm(
            &session_key,
            &server_sectkn,
            b"APPUSER",
            b"1234567890123456",
            EncryptionAlgorithm::Aes,
        );
        assert_eq!(ct2.len(), 32);
    }

    #[test]
    fn test_secmec7_aes_uses_server_token_iv_not_userid_iv() {
        let session_key = vec![0x11; 64];
        let server_sectkn_a = vec![0x22; 64];
        let mut server_sectkn_b = vec![0x22; 64];
        server_sectkn_b[24] = 0x33;

        let ct_user_a = encrypt_password_with_userid_iv_bytes_with_algorithm(
            &session_key,
            &server_sectkn_a,
            b"APPUSERA",
            b"password",
            EncryptionAlgorithm::Aes,
        );
        let ct_user_b = encrypt_password_with_userid_iv_bytes_with_algorithm(
            &session_key,
            &server_sectkn_a,
            b"APPUSERB",
            b"password",
            EncryptionAlgorithm::Aes,
        );
        let ct_server_b = encrypt_password_with_userid_iv_bytes_with_algorithm(
            &session_key,
            &server_sectkn_b,
            b"APPUSERA",
            b"password",
            EncryptionAlgorithm::Aes,
        );

        assert_eq!(ct_user_a, ct_user_b);
        assert_ne!(ct_user_a, ct_server_b);
    }

    #[test]
    fn test_secmec7_aes_matches_ibm_jcc_vector() {
        let session_key = hex_to_bytes(
            "73d52e0f878d6e6c5361f9f6ef31d8796161cef670be34d9345bffce7cf5ad81\
             608d642b93b26eca6b86787cf156dc6d011b04ab17f803545f3fbd655e4470cb",
        );
        let server_sectkn = hex_to_bytes(
            "632c09324e3b451c622865a8a1897070f26cc3d3920560ae862a6d9a733ee1f1\
             7ec53d148202264972b41632cf4a05165d26437182236438289923c557050eae",
        );
        let encrypted = encrypt_password_with_userid_iv_bytes_with_algorithm(
            &session_key,
            &server_sectkn,
            b"ignored",
            b"password",
            EncryptionAlgorithm::Aes,
        );

        assert_eq!(encrypted, hex_to_bytes("b7e8bb87c141d97d93a1f7e60c61c50c"));
    }

    #[test]
    fn test_generate_private_key_in_range() {
        let key = generate_private_key();
        assert_eq!(key.len(), 32);
        let k = U256::from_bytes(&key);
        let prime = U256::from_bytes(&DH_PRIME);
        assert!(k.cmp(&prime) < 0, "private key must be less than prime");
        assert!(!k.is_zero(), "private key must not be zero");
    }
}
