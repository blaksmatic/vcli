#[path = "../build_support.rs"]
mod build_support;

#[test]
fn command_line_tools_developer_dir_yields_swift55_rpath() {
    let dir = std::path::Path::new("/Library/Developer/CommandLineTools");
    let paths = build_support::swift55_macosx_rpath_candidates(Some(dir));

    assert_eq!(paths[0], std::path::PathBuf::from("/usr/lib/swift"));
    assert_eq!(
        paths[1],
        std::path::PathBuf::from("/Library/Developer/CommandLineTools/usr/lib/swift-5.5/macosx")
    );
}

#[test]
fn xcode_developer_dir_yields_toolchain_swift55_rpath() {
    let dir = std::path::Path::new("/Applications/Xcode.app/Contents/Developer");
    let paths = build_support::swift55_macosx_rpath_candidates(Some(dir));

    assert_eq!(paths[0], std::path::PathBuf::from("/usr/lib/swift"));
    assert_eq!(
        paths[1],
        std::path::PathBuf::from(
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx"
        )
    );
}

#[test]
fn candidates_are_deduped() {
    let dir = std::path::Path::new("/Library/Developer/CommandLineTools");
    let paths = build_support::swift55_macosx_rpath_candidates(Some(dir));
    let set: std::collections::BTreeSet<_> = paths.iter().collect();

    assert_eq!(paths.len(), set.len());
}

#[test]
fn developer_dir_probe_is_optional() {
    let _ = build_support::selected_developer_dir();
}

#[test]
fn existing_rpath_probe_is_optional() {
    let _ = build_support::swift_runtime_rpaths_to_emit();
}
