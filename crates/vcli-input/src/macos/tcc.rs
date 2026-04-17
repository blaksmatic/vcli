//! macOS TCC probes. Uses `AXIsProcessTrustedWithOptions` for Accessibility
//! and `IOHIDCheckAccess` for Input Monitoring. No prompts are triggered —
//! the options dictionary passes `kAXTrustedCheckOptionPrompt: false`.

#![allow(unsafe_code)]

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;

use crate::permissions::{PermissionReport, PermissionStatus};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef)
        -> bool;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDCheckAccess(request: u32) -> u32; // IOHIDRequestType: 0=PostEvent, 1=ListenEvent
}

// Values from IOKit IOHIDLib.h: kIOHIDAccessType{Granted=0,Denied=1,Unknown=2}.
const IOHID_ACCESS_GRANTED: u32 = 0;
const IOHID_ACCESS_DENIED: u32 = 1;
const IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;

/// Full report (Accessibility + Input Monitoring).
#[must_use]
pub fn probe_report() -> PermissionReport {
    PermissionReport {
        accessibility: accessibility_status(),
        input_monitoring: input_monitoring_status(),
    }
}

fn accessibility_status() -> PermissionStatus {
    // Build {"AXTrustedCheckOptionPrompt": false} dictionary without prompting.
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::false_value();
    let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let trusted = unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef()) };
    if trusted {
        PermissionStatus::Granted
    } else {
        // AX API doesn't distinguish Denied vs NotDetermined. Report
        // NotDetermined so the caller prompts the user the first time.
        PermissionStatus::NotDetermined
    }
}

fn input_monitoring_status() -> PermissionStatus {
    let status = unsafe { IOHIDCheckAccess(IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    match status {
        IOHID_ACCESS_GRANTED => PermissionStatus::Granted,
        IOHID_ACCESS_DENIED => PermissionStatus::Denied,
        _ => PermissionStatus::NotDetermined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_does_not_panic() {
        // In CI Input Monitoring will typically be NotDetermined and
        // Accessibility will typically be NotDetermined — we only assert we
        // got SOME PermissionReport value.
        let r = probe_report();
        let _ = serde_json::to_string(&r).unwrap();
    }
}
