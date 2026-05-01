#[cfg(target_os = "macos")]
#[path = "../vcli-capture/build_support.rs"]
mod build_support;

fn main() {
    #[cfg(target_os = "macos")]
    {
        // vcli-daemon pulls in vcli-capture → screencapturekit transitively.
        // Add the same Swift runtime rpaths so daemon test + binary artifacts
        // can load screencapturekit's Swift bridge.
        for path in build_support::swift_runtime_rpaths_to_emit() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", path.display());
        }
    }
}
