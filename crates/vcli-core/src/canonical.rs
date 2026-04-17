//! Canonical JSON serialization for stable hashing. Decision 1.1.
//!
//! The function `canonicalize` takes any `serde_json::Value` and emits a
//! `Vec<u8>` of canonical bytes. Two semantically-equal values MUST produce
//! identical bytes. `predicate_hash` is `sha256(canonicalize(value))` wrapped
//! in the `PredicateHash` newtype.

use std::fmt::{self, Write as FmtWrite};
use std::io::{self, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Canonicalization error.
#[derive(Debug, Error)]
pub enum CanonicalError {
    /// IO error while writing.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Serialize `value` into canonical bytes.
///
/// Rules:
/// - Object keys sorted lexicographically by UTF-8 bytes.
/// - Numbers: integers as plain decimal (no leading zeros, `-0` normalized to `0`);
///   non-integers via `ryu`.
/// - Strings: UTF-8 NFC-normalized, then JSON-escaped (minimal escapes).
/// - No whitespace anywhere.
///
/// # Errors
///
/// Returns `CanonicalError::Io` only if the underlying writer fails, which
/// cannot happen for the `Vec<u8>` we use internally — but the API surfaces
/// the `Result` for future generalization.
pub fn canonicalize(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut out = Vec::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(w: &mut Vec<u8>, v: &Value) -> io::Result<()> {
    match v {
        Value::Null => w.write_all(b"null"),
        Value::Bool(true) => w.write_all(b"true"),
        Value::Bool(false) => w.write_all(b"false"),
        Value::Number(n) => write_number(w, n),
        Value::String(s) => write_string(w, s),
        Value::Array(a) => write_array(w, a),
        Value::Object(m) => write_object(w, m),
    }
}

fn write_number(w: &mut Vec<u8>, n: &serde_json::Number) -> io::Result<()> {
    if let Some(i) = n.as_i64() {
        // Plain decimal. `-0` → `0`.
        let s = if i == 0 { "0".to_string() } else { i.to_string() };
        w.write_all(s.as_bytes())
    } else if let Some(u) = n.as_u64() {
        w.write_all(u.to_string().as_bytes())
    } else if let Some(f) = n.as_f64() {
        if !f.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "canonical JSON: non-finite number",
            ));
        }
        let f_norm = if f == 0.0 { 0.0 } else { f };
        let mut buf = ryu::Buffer::new();
        w.write_all(buf.format(f_norm).as_bytes())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "unrepresentable number"))
    }
}

fn write_string(w: &mut Vec<u8>, s: &str) -> io::Result<()> {
    let normalized: String = s.nfc().collect();
    w.write_all(b"\"")?;
    for ch in normalized.chars() {
        match ch {
            '"' => w.write_all(b"\\\"")?,
            '\\' => w.write_all(b"\\\\")?,
            '\n' => w.write_all(b"\\n")?,
            '\r' => w.write_all(b"\\r")?,
            '\t' => w.write_all(b"\\t")?,
            '\x08' => w.write_all(b"\\b")?,
            '\x0c' => w.write_all(b"\\f")?,
            c if (c as u32) < 0x20 => {
                let s = format!("\\u{:04x}", c as u32);
                w.write_all(s.as_bytes())?;
            }
            c => {
                let mut buf = [0u8; 4];
                w.write_all(c.encode_utf8(&mut buf).as_bytes())?;
            }
        }
    }
    w.write_all(b"\"")
}

fn write_array(w: &mut Vec<u8>, arr: &[Value]) -> io::Result<()> {
    w.write_all(b"[")?;
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        write_value(w, v)?;
    }
    w.write_all(b"]")
}

fn write_object(w: &mut Vec<u8>, obj: &serde_json::Map<String, Value>) -> io::Result<()> {
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    w.write_all(b"{")?;
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        write_string(w, k)?;
        w.write_all(b":")?;
        write_value(w, obj.get(*k).unwrap())?;
    }
    w.write_all(b"}")
}

/// 32-byte SHA-256 over canonical bytes, hex-encoded.
///
/// We implement SHA-256 manually using a tiny pure-Rust rotate-and-add loop
/// to avoid pulling in a crypto crate just for this. For production use the
/// Rust `sha2` crate is preferable — but this crate is meant to be tiny and
/// dep-light; `sha2` can be introduced later if needed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PredicateHash(String);

impl PredicateHash {
    /// Hex string form (`"sha256:<hex>"` is NOT used here — that prefix lives in
    /// the daemon's asset-reference strings. This type is just the hash bytes).
    #[must_use]
    pub fn hex(&self) -> &str {
        &self.0
    }

    /// Wrap a precomputed hex hash. Caller is responsible for correctness.
    #[must_use]
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }
}

impl fmt::Display for PredicateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Hash a `Value` by canonicalizing and SHA-256-ing.
///
/// # Errors
///
/// Returns `CanonicalError::Io` only on writer failure (effectively unreachable).
pub fn predicate_hash(value: &Value) -> Result<PredicateHash, CanonicalError> {
    let bytes = canonicalize(value)?;
    Ok(PredicateHash(sha256_hex(&bytes)))
}

// ---- minimal SHA-256 (FIPS-180-4) ----------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
        0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
    ];
    let k: [u32; 64] = [
        0x428a_2f98,0x7137_4491,0xb5c0_fbcf,0xe9b5_dba5,0x3956_c25b,0x59f1_11f1,0x923f_82a4,0xab1c_5ed5,
        0xd807_aa98,0x1283_5b01,0x2431_85be,0x550c_7dc3,0x72be_5d74,0x80de_b1fe,0x9bdc_06a7,0xc19b_f174,
        0xe49b_69c1,0xefbe_4786,0x0fc1_9dc6,0x240c_a1cc,0x2de9_2c6f,0x4a74_84aa,0x5cb0_a9dc,0x76f9_88da,
        0x983e_5152,0xa831_c66d,0xb003_27c8,0xbf59_7fc7,0xc6e0_0bf3,0xd5a7_9147,0x06ca_6351,0x1429_2967,
        0x27b7_0a85,0x2e1b_2138,0x4d2c_6dfc,0x5338_0d13,0x650a_7354,0x766a_0abb,0x81c2_c92e,0x9272_2c85,
        0xa2bf_e8a1,0xa81a_664b,0xc24b_8b70,0xc76c_51a3,0xd192_e819,0xd699_0624,0xf40e_3585,0x106a_a070,
        0x19a4_c116,0x1e37_6c08,0x2748_774c,0x34b0_bcb5,0x391c_0cb3,0x4ed8_aa4a,0x5b9c_ca4f,0x682e_6ff3,
        0x748f_82ee,0x78a5_636f,0x84c8_7814,0x8cc7_0208,0x90be_fffa,0xa450_6ceb,0xbef9_a3f7,0xc671_78f2,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut h = state;
        for i in 0..64 {
            let s1 = h[4].rotate_right(6) ^ h[4].rotate_right(11) ^ h[4].rotate_right(25);
            let ch = (h[4] & h[5]) ^ (!h[4] & h[6]);
            let temp1 = h[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k[i])
                .wrapping_add(w[i]);
            let s0 = h[0].rotate_right(2) ^ h[0].rotate_right(13) ^ h[0].rotate_right(22);
            let maj = (h[0] & h[1]) ^ (h[0] & h[2]) ^ (h[1] & h[2]);
            let temp2 = s0.wrapping_add(maj);
            h[7] = h[6];
            h[6] = h[5];
            h[5] = h[4];
            h[4] = h[3].wrapping_add(temp1);
            h[3] = h[2];
            h[2] = h[1];
            h[1] = h[0];
            h[0] = temp1.wrapping_add(temp2);
        }
        for (i, val) in h.iter().enumerate() {
            state[i] = state[i].wrapping_add(*val);
        }
    }
    let mut s = String::with_capacity(64);
    for word in state {
        write!(s, "{word:08x}").unwrap();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_sorted_lexicographically() {
        let v = json!({"b": 1, "a": 2, "c": {"z": 1, "a": 2}});
        let out = canonicalize(&v).unwrap();
        assert_eq!(out, br#"{"a":2,"b":1,"c":{"a":2,"z":1}}"#);
    }

    #[test]
    fn integers_as_plain_decimal() {
        let v = json!({"n": 42, "z": 0, "neg": -7});
        let out = canonicalize(&v).unwrap();
        assert_eq!(out, br#"{"n":42,"neg":-7,"z":0}"#);
    }

    #[test]
    fn floats_via_ryu() {
        let v = json!({"x": 0.1});
        let out = canonicalize(&v).unwrap();
        // ryu emits shortest round-trip form.
        assert_eq!(out, br#"{"x":0.1}"#);
    }

    #[test]
    fn empty_containers_emit_correctly() {
        assert_eq!(canonicalize(&json!([])).unwrap(), b"[]");
        assert_eq!(canonicalize(&json!({})).unwrap(), b"{}");
        assert_eq!(canonicalize(&json!(null)).unwrap(), b"null");
    }

    #[test]
    fn strings_escape_minimally() {
        let v = json!("hello\nworld\t\"yes\"");
        assert_eq!(
            canonicalize(&v).unwrap(),
            br#""hello\nworld\t\"yes\"""#
        );
    }

    #[test]
    fn nfc_normalization_applied_to_strings() {
        // Combining sequence "e" + U+0301 must normalize to precomposed "é".
        let decomposed = "e\u{0301}";
        let composed = "\u{00e9}";
        let a = canonicalize(&json!(decomposed)).unwrap();
        let b = canonicalize(&json!(composed)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn same_semantic_value_same_hash() {
        let a = json!({"b": 1, "a": 2});
        let b = json!({"a": 2, "b": 1});
        assert_eq!(predicate_hash(&a).unwrap(), predicate_hash(&b).unwrap());
    }

    #[test]
    fn different_values_different_hash() {
        let a = json!({"a": 1});
        let b = json!({"a": 2});
        assert_ne!(predicate_hash(&a).unwrap(), predicate_hash(&b).unwrap());
    }

    #[test]
    fn sha256_matches_known_vector_empty() {
        // Known: sha256("") = e3b0c442 98fc1c14 9afbf4c8 996fb924 27ae41e4 649b934c a495991b 7852b855
        let h = sha256_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_matches_known_vector_abc() {
        // Known: sha256("abc") = ba7816bf 8f01cfea 414140de 5dae2223 b00361a3 96177a9c b410ff61 f20015ad
        let h = sha256_hex(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn predicate_hash_hex_is_64_chars() {
        let h = predicate_hash(&json!({"x": 1})).unwrap();
        assert_eq!(h.hex().len(), 64);
        assert!(h.hex().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
