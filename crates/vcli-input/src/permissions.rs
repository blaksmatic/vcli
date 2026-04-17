//! Permission diagnostics. On macOS, input synthesis needs Accessibility AND
//! (for some event types) Input Monitoring TCC buckets granted. Reports a
//! status per bucket without prompting (diagnostic only).

use serde::{Deserialize, Serialize};

/// Status of a single TCC (or equivalent) permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionStatus {
    /// Granted — input synthesis works.
    Granted,
    /// Denied — user explicitly refused.
    Denied,
    /// Not determined — never asked. Typically means granted in practice for
    /// Accessibility if the user has not yet opened the toggle, so dispatch
    /// will prompt / fail the first time.
    NotDetermined,
    /// Platform does not use this concept (always returned on non-macOS).
    NotApplicable,
}

/// Aggregate report printed by `vcli health` and the daemon readiness check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionReport {
    /// macOS Accessibility bucket (required for `CGEventPost`).
    pub accessibility: PermissionStatus,
    /// macOS Input Monitoring bucket (required for the `CGEventTap` listener
    /// used by the kill-switch hotkey).
    pub input_monitoring: PermissionStatus,
}

impl PermissionReport {
    /// True iff both buckets are `Granted`.
    #[must_use]
    pub fn fully_granted(&self) -> bool {
        matches!(self.accessibility, PermissionStatus::Granted)
            && matches!(self.input_monitoring, PermissionStatus::Granted)
    }
}

/// Probe permissions. On macOS this calls into `macos::tcc`; on any other OS
/// both buckets report `NotApplicable`.
#[must_use]
pub fn probe() -> PermissionReport {
    #[cfg(target_os = "macos")]
    {
        crate::macos::tcc::probe_report()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermissionReport {
            accessibility: PermissionStatus::NotApplicable,
            input_monitoring: PermissionStatus::NotApplicable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrip() {
        let r = PermissionReport {
            accessibility: PermissionStatus::Granted,
            input_monitoring: PermissionStatus::Denied,
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains(r#""accessibility":"granted""#));
        assert!(j.contains(r#""input_monitoring":"denied""#));
        let back: PermissionReport = serde_json::from_str(&j).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn fully_granted_requires_both() {
        assert!(PermissionReport {
            accessibility: PermissionStatus::Granted,
            input_monitoring: PermissionStatus::Granted,
        }
        .fully_granted());
        assert!(!PermissionReport {
            accessibility: PermissionStatus::Granted,
            input_monitoring: PermissionStatus::Denied,
        }
        .fully_granted());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_macos_reports_not_applicable() {
        let r = probe();
        assert_eq!(r.accessibility, PermissionStatus::NotApplicable);
        assert_eq!(r.input_monitoring, PermissionStatus::NotApplicable);
    }
}
