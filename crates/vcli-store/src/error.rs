//! Error types for the store.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Error surface for every fallible `Store` operation.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite returned an error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Filesystem IO error (asset store, db path creation).
    #[error("io at {path:?}: {source}")]
    Io {
        /// Path that triggered the IO error, if known.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: io::Error,
    },

    /// A program row was not found.
    #[error("unknown program: {0}")]
    UnknownProgram(String),

    /// Requested state transition is not allowed from the current state.
    #[error("bad state transition: {from} -> {to} ({reason})")]
    BadStateTransition {
        /// Starting state.
        from: String,
        /// Requested ending state.
        to: String,
        /// Human-readable reason.
        reason: String,
    },

    /// Program was not failed(daemon_restart) or is otherwise not resumable.
    #[error("not resumable: {0}")]
    NotResumable(String),

    /// Asset bytes not found for the given hash.
    #[error("unknown asset: {0}")]
    UnknownAsset(String),

    /// Schema version in the db is newer than this binary understands.
    #[error("schema version {found} is newer than supported version {supported}")]
    SchemaNewer {
        /// Version read from the db.
        found: u32,
        /// Highest version baked into this binary.
        supported: u32,
    },

    /// A stored JSON blob failed to deserialize.
    #[error("json deserialize: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience alias.
pub type StoreResult<T> = Result<T, StoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_program_display() {
        let e = StoreError::UnknownProgram("abc".into());
        assert_eq!(e.to_string(), "unknown program: abc");
    }

    #[test]
    fn bad_state_transition_carries_fields() {
        let e = StoreError::BadStateTransition {
            from: "completed".into(),
            to: "running".into(),
            reason: "terminal".into(),
        };
        assert!(e.to_string().contains("completed -> running"));
    }

    #[test]
    fn schema_newer_display() {
        let e = StoreError::SchemaNewer { found: 5, supported: 3 };
        assert!(e.to_string().contains("5"));
        assert!(e.to_string().contains("3"));
    }
}
