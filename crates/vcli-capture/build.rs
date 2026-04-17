fn main() {
    // Link CoreGraphics framework for CGPreflightScreenCaptureAccess and
    // CGRequestScreenCaptureAccess used in permission.rs on macOS.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
}
