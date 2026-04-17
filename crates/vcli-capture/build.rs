fn main() {
    // Link CoreGraphics framework for CGPreflightScreenCaptureAccess and
    // CGRequestScreenCaptureAccess used in permission.rs on macOS.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreGraphics");

        // The screencapturekit crate (v1.5.4) uses a Swift bridge that references
        // libswift_Concurrency.dylib. On macOS 26+ this dylib is not a physical
        // file but lives in the dyld shared cache. On older macOS it lives in
        // the Swift toolchain (e.g., Xcode's swift-5.5 directory). Add the
        // Xcode toolchain path to rpath so the loader finds it on both.
        let xcode_swift55_path =
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain\
             /usr/lib/swift-5.5/macosx";
        println!("cargo:rustc-link-arg=-Wl,-rpath,{xcode_swift55_path}");
    }
}
