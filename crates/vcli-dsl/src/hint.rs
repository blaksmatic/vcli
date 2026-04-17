//! Tiny Levenshtein-1 did-you-mean helper used by the validator to offer
//! suggestions when a name is misspelled by a single edit.

/// Return the first name in `candidates` whose Levenshtein distance from
/// `needle` is at most 1, case-sensitive. `None` if none are close enough or
/// `needle` is an exact match.
pub fn did_you_mean<'a, I, S>(needle: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str> + 'a,
{
    for c in candidates {
        let c = c.as_ref();
        if c == needle {
            return None;
        }
        if distance_at_most_1(needle, c) {
            return Some(c.to_string());
        }
    }
    None
}

fn distance_at_most_1(a: &str, b: &str) -> bool {
    let la = a.chars().count();
    let lb = b.chars().count();
    match la.cmp(&lb) {
        std::cmp::Ordering::Equal => {
            let mut diff = 0usize;
            for (ca, cb) in a.chars().zip(b.chars()) {
                if ca != cb {
                    diff += 1;
                    if diff > 1 {
                        return false;
                    }
                }
            }
            diff <= 1
        }
        std::cmp::Ordering::Less => is_insertion(a, b),
        std::cmp::Ordering::Greater => is_insertion(b, a),
    }
}

fn is_insertion(shorter: &str, longer: &str) -> bool {
    if longer.chars().count() != shorter.chars().count() + 1 {
        return false;
    }
    let mut si = shorter.chars().peekable();
    let mut found_gap = false;
    for lc in longer.chars() {
        match si.peek() {
            Some(&sc) if sc == lc => {
                si.next();
            }
            _ => {
                if found_gap {
                    return false;
                }
                found_gap = true;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_returns_none() {
        assert_eq!(did_you_mean("skip", &["skip", "nope"] as &[&str]), None);
    }

    #[test]
    fn one_substitution_detected() {
        assert_eq!(
            did_you_mean("skp_visible", &["skip_visible", "other"] as &[&str]),
            Some("skip_visible".into())
        );
    }

    #[test]
    fn one_insertion_detected() {
        assert_eq!(
            did_you_mean("sip", &["skip"] as &[&str]),
            Some("skip".into())
        );
    }

    #[test]
    fn one_deletion_detected() {
        assert_eq!(
            did_you_mean("skipp", &["skip"] as &[&str]),
            Some("skip".into())
        );
    }

    #[test]
    fn two_edits_rejected() {
        assert_eq!(did_you_mean("skap", &["skip_visible"] as &[&str]), None);
    }

    #[test]
    fn first_match_wins() {
        assert_eq!(
            did_you_mean("ab", &["ax", "ay"] as &[&str]),
            Some("ax".into())
        );
    }
}
