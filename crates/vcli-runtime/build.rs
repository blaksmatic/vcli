fn main() {
    #[cfg(target_os = "macos")]
    {
        // vcli-runtime pulls in vcli-capture → screencapturekit transitively.
        // screencapturekit's Swift bridge references libswift_Concurrency.dylib.
        // vcli-capture sets this rpath for its own artifacts; we repeat it here
        // so vcli-runtime's test + binary output can load the Swift dylib too.
        let xcode_swift55_path =
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain\
             /usr/lib/swift-5.5/macosx";
        println!("cargo:rustc-link-arg=-Wl,-rpath,{xcode_swift55_path}");
    }
}
