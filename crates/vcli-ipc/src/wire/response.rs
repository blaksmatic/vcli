//! Response envelope placeholder — implemented in Task 7.

use serde::{Deserialize, Serialize};

use vcli_core::ErrorPayload;

use super::request::RequestId;

/// Server response to a single `Request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Correlation id copied from the originating `Request`.
    pub id: RequestId,
    /// Body discriminated by `ok`.
    #[serde(flatten)]
    pub body: ResponseBody,
}

/// Body variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    /// Success variant. `ok: true`.
    Ok {
        /// Discriminant literal `true`.
        ok: OkFlag,
        /// Op-specific result payload.
        result: serde_json::Value,
    },
    /// Error variant. `ok: false`.
    Err {
        /// Discriminant literal `false`.
        ok: ErrFlag,
        /// Typed error payload.
        error: ErrorPayload,
    },
}

/// Phantom-ish type that (de)serializes only as literal `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OkFlag;

impl Serialize for OkFlag {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(true)
    }
}

impl<'de> Deserialize<'de> for OkFlag {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let b = bool::deserialize(d)?;
        if b {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("expected true"))
        }
    }
}

/// Phantom-ish type that (de)serializes only as literal `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrFlag;

impl Serialize for ErrFlag {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(false)
    }
}

impl<'de> Deserialize<'de> for ErrFlag {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let b = bool::deserialize(d)?;
        if b {
            Err(serde::de::Error::custom("expected false"))
        } else {
            Ok(Self)
        }
    }
}

impl Response {
    /// Build an `Ok` response.
    #[must_use]
    pub fn ok(id: RequestId, result: serde_json::Value) -> Self {
        Self {
            id,
            body: ResponseBody::Ok { ok: OkFlag, result },
        }
    }

    /// Build an `Err` response.
    #[must_use]
    pub fn err(id: RequestId, error: ErrorPayload) -> Self {
        Self {
            id,
            body: ResponseBody::Err { ok: ErrFlag, error },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcli_core::ErrorCode;

    fn rid() -> RequestId {
        "12345678-1234-4567-8910-111213141516".parse().unwrap()
    }

    #[test]
    fn ok_response_serializes_with_ok_true() {
        let r = Response::ok(rid(), serde_json::json!({ "program_id": "abc" }));
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""ok":true"#), "{j}");
        assert!(j.contains(r#""program_id":"abc""#), "{j}");
        assert!(!j.contains("error"));
        let back: Response = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn err_response_serializes_with_ok_false_and_typed_error() {
        let r = Response::err(
            rid(),
            ErrorPayload::simple(ErrorCode::UnknownProgram, "not found"),
        );
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""ok":false"#), "{j}");
        assert!(j.contains(r#""code":"unknown_program""#), "{j}");
        assert!(!j.contains(r#""result""#), "{j}");
        let back: Response = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn ok_response_with_null_result_is_valid() {
        let r = Response::ok(rid(), serde_json::Value::Null);
        let j = serde_json::to_string(&r).unwrap();
        let back: Response = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn response_with_ok_flag_mismatch_rejected() {
        // ok:true paired with error field → must fail deserialization (no matching variant).
        let bad = r#"{"id":"12345678-1234-4567-8910-111213141516","ok":true,"error":{"code":"internal","message":"x"}}"#;
        assert!(serde_json::from_str::<Response>(bad).is_err());
    }
}
