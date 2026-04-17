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
        if !b {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("expected false"))
        }
    }
}

impl Response {
    /// Build an `Ok` response.
    #[must_use]
    pub fn ok(id: RequestId, result: serde_json::Value) -> Self {
        Self { id, body: ResponseBody::Ok { ok: OkFlag, result } }
    }

    /// Build an `Err` response.
    #[must_use]
    pub fn err(id: RequestId, error: ErrorPayload) -> Self {
        Self { id, body: ResponseBody::Err { ok: ErrFlag, error } }
    }
}
