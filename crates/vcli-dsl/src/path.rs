//! JSON-pointer-style paths (RFC 6901) for pointing at specific fields of the
//! program document in errors. Kept small and immutable-friendly for cheap
//! cloning during validation.

use std::fmt;

/// Accumulated path into the program. `/predicates/skip_visible/region/predicate`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonPath {
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(usize),
}

impl JsonPath {
    /// Empty root path.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// Append an object key.
    #[must_use]
    pub fn key(&self, k: impl Into<String>) -> Self {
        let mut s = self.clone();
        s.segments.push(Segment::Key(k.into()));
        s
    }

    /// Append an array index.
    #[must_use]
    pub fn index(&self, i: usize) -> Self {
        let mut s = self.clone();
        s.segments.push(Segment::Index(i));
        s
    }

    /// True when this is the document root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }
}

impl fmt::Display for JsonPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.segments.is_empty() {
            return f.write_str("");
        }
        for seg in &self.segments {
            match seg {
                Segment::Key(k) => {
                    f.write_str("/")?;
                    for ch in k.chars() {
                        match ch {
                            '~' => f.write_str("~0")?,
                            '/' => f.write_str("~1")?,
                            c => f.write_fmt(format_args!("{c}"))?,
                        }
                    }
                }
                Segment::Index(i) => f.write_fmt(format_args!("/{i}"))?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_empty_string() {
        assert_eq!(JsonPath::root().to_string(), "");
        assert!(JsonPath::root().is_root());
    }

    #[test]
    fn key_then_index_then_key_roundtrip() {
        let p = JsonPath::root().key("watches").index(1).key("when");
        assert_eq!(p.to_string(), "/watches/1/when");
    }

    #[test]
    fn special_chars_in_keys_are_escaped() {
        let p = JsonPath::root().key("a/b").key("c~d");
        assert_eq!(p.to_string(), "/a~1b/c~0d");
    }

    #[test]
    fn cloning_keeps_segments_independent() {
        let base = JsonPath::root().key("predicates");
        let a = base.key("foo");
        let b = base.key("bar");
        assert_eq!(a.to_string(), "/predicates/foo");
        assert_eq!(b.to_string(), "/predicates/bar");
        assert_eq!(base.to_string(), "/predicates");
    }
}
