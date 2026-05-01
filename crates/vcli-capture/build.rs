#[cfg(target_os = "macos")]
mod build_support;

fn main() {
    // Link CoreGraphics framework for CGPreflightScreenCaptureAccess and
    // CGRequestScreenCaptureAccess used in permission.rs on macOS.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreGraphics");

        // The screencapturekit crate (v1.5.4) uses a Swift bridge that
        // references libswift_Concurrency.dylib via @rpath. Prefer the OS
        // Swift runtime path, then fall back to the selected developer
        // toolchain's Swift 5.5 runtime for older installations.
        for path in build_support::swift_runtime_rpaths_to_emit() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", path.display());
        }
    }
}
