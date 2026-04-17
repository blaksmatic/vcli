//! DSL `Step` — the vocabulary shared by `watches[*].do` and `body`.
//! Inputs carry resolvable expressions (`$pred.match.center`) rather than
//! concrete points — that resolution happens in `vcli-runtime` before
//! producing an `InputAction` for dispatch.

use serde::{Deserialize, Serialize};

use crate::action::{Button, Modifier};

/// Target of a step that interacts with a screen location. Either a concrete
/// point (for absolute coordinates) or an expression string like
/// `"$skip_visible.match.center"` resolved at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    /// Absolute point. `{"x": 100, "y": 200}`.
    Absolute(crate::geom::Point),
    /// Expression. `"$p.match.center"`.
    Expression(String),
}

/// What happens when a `wait_for` step's predicate never becomes truthy
/// before `timeout_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnTimeout {
    /// Program transitions to `failed`.
    Fail,
    /// Skip the wait and continue to the next body step.
    Continue,
    /// Re-evaluate the predicate one more tick. (v0: equivalent to one extra tick; no backoff.)
    Retry,
}

/// What happens when an `assert` fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    /// Program transitions to `failed`.
    Fail,
    /// Skip and continue to the next body step. Useful for best-effort checks.
    Continue,
}

/// A DSL step. Used in both `body` (sequential) and `watches[*].do` (reactive).
/// Control-flow variants (`WaitFor`, `Assert`, `SleepMs`) are body-only; the
/// validator (`vcli-dsl`) rejects them in watches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// Move cursor.
    Move {
        /// Destination.
        at: Target,
    },
    /// Click at a target.
    Click {
        /// Click target.
        at: Target,
        /// Which button to click with.
        #[serde(default = "default_button")]
        button: Button,
    },
    /// Type literal text.
    Type {
        /// Text.
        text: String,
    },
    /// Press a key combo.
    Key {
        /// Key name.
        key: String,
        /// Modifier keys held during the press.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        modifiers: Vec<Modifier>,
    },
    /// Scroll at a target.
    Scroll {
        /// Scroll target.
        at: Target,
        /// Horizontal delta.
        #[serde(default)]
        dx: i32,
        /// Vertical delta.
        #[serde(default)]
        dy: i32,
    },

    /// Body-only. Block until predicate becomes truthy or timeout fires.
    WaitFor {
        /// Predicate name.
        predicate: String,
        /// Max milliseconds to wait.
        timeout_ms: u32,
        /// Behavior on timeout.
        #[serde(default = "default_on_timeout")]
        on_timeout: OnTimeout,
    },
    /// Body-only. Fail the program (or continue) if the named predicate is not truthy.
    Assert {
        /// Predicate name.
        predicate: String,
        /// Behavior on failure.
        #[serde(default = "default_on_fail")]
        on_fail: OnFail,
    },
    /// Body-only. Sleep for a fixed duration. NOT resumable (see Decision C).
    SleepMs {
        /// Milliseconds.
        ms: u32,
    },
}

fn default_button() -> Button {
    Button::Left
}
fn default_on_timeout() -> OnTimeout {
    OnTimeout::Fail
}
fn default_on_fail() -> OnFail {
    OnFail::Fail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Point;

    #[test]
    fn click_with_expression_target() {
        let j = r#"{"kind":"click","at":"$skip.match.center"}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        assert_eq!(
            s,
            Step::Click {
                at: Target::Expression("$skip.match.center".into()),
                button: Button::Left,
            }
        );
    }

    #[test]
    fn click_with_absolute_target() {
        let j = r#"{"kind":"click","at":{"x":10,"y":20}}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        assert_eq!(
            s,
            Step::Click {
                at: Target::Absolute(Point { x: 10, y: 20 }),
                button: Button::Left,
            }
        );
    }

    #[test]
    fn click_button_roundtrip() {
        let s = Step::Click {
            at: Target::Expression("$p.match.center".into()),
            button: Button::Right,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: Step = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn wait_for_defaults_to_fail_on_timeout() {
        let j = r#"{"kind":"wait_for","predicate":"p","timeout_ms":1000}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        match s {
            Step::WaitFor { on_timeout, .. } => assert_eq!(on_timeout, OnTimeout::Fail),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn assert_defaults_to_fail_on_fail() {
        let j = r#"{"kind":"assert","predicate":"p"}"#;
        let s: Step = serde_json::from_str(j).unwrap();
        match s {
            Step::Assert { on_fail, .. } => assert_eq!(on_fail, OnFail::Fail),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn sleep_ms_roundtrip() {
        let s = Step::SleepMs { ms: 250 };
        let j = serde_json::to_string(&s).unwrap();
        let back: Step = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn type_and_key_and_scroll_roundtrips() {
        for s in [
            Step::Type { text: "hi".into() },
            Step::Key {
                key: "s".into(),
                modifiers: vec![Modifier::Cmd],
            },
            Step::Scroll {
                at: Target::Expression("$p.match.center".into()),
                dx: 0,
                dy: -40,
            },
        ] {
            let back: Step = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(back, s);
        }
    }
}
