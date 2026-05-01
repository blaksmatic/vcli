use std::path::{Path, PathBuf};
use std::process::Command;

/// Return the active Apple developer directory selected by `xcode-select`.
pub fn selected_developer_dir() -> Option<PathBuf> {
    let out = Command::new("xcode-select").arg("-p").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8(out.stdout).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Candidate locations for the Swift 5.5 macOS runtime directory.
pub fn swift55_macosx_rpath_candidates(selected_developer_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    push_unique(&mut out, PathBuf::from("/usr/lib/swift"));
    if let Some(dir) = selected_developer_dir {
        if dir.ends_with("CommandLineTools") {
            push_unique(&mut out, dir.join("usr/lib/swift-5.5/macosx"));
        } else {
            push_unique(
                &mut out,
                dir.join("Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx"),
            );
            push_unique(&mut out, dir.join("usr/lib/swift-5.5/macosx"));
        }
    }
    push_unique(
        &mut out,
        PathBuf::from("/Library/Developer/CommandLineTools/usr/lib/swift-5.5/macosx"),
    );
    push_unique(
        &mut out,
        PathBuf::from(
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx",
        ),
    );
    out
}

/// Swift runtime directories to emit as rpaths.
pub fn swift_runtime_rpaths_to_emit() -> Vec<PathBuf> {
    swift55_macosx_rpath_candidates(selected_developer_dir().as_deref())
        .into_iter()
        .filter(|p| {
            p == Path::new("/usr/lib/swift") || p.join("libswift_Concurrency.dylib").exists()
        })
        .collect()
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|p| p == &path) {
        paths.push(path);
    }
}
