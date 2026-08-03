use super::{
    bundle_identifier_for_executable_path, desktop_identity_matches_instance,
};

#[test]
fn live_bundled_sibling_matches_without_procargs_identifier() {
    // A directly executed app binary receives neither Tauri's --config argv nor
    // LaunchServices' __CFBundleIdentifier environment entry. Its signed bundle
    // metadata still identifies the live owner, so it must not be reaped.
    assert!(desktop_identity_matches_instance(
        b"/Applications/Buzz Proof.app/Contents/MacOS/Buzz\0",
        Some("xyz.block.buzz.app.dev.proof"),
        "xyz.block.buzz.app.dev.proof",
    ));
}

#[test]
fn direct_executable_resolves_its_bundle_identifier() {
    let temp = tempfile::tempdir().expect("tempdir");
    let contents = temp.path().join("Buzz Proof.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).expect("create bundle layout");
    std::fs::write(
        contents.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>xyz.block.buzz.app.dev.proof</string>
</dict></plist>"#,
    )
    .expect("write Info.plist");

    assert_eq!(
        bundle_identifier_for_executable_path(&contents.join("MacOS/Buzz")).as_deref(),
        Some("xyz.block.buzz.app.dev.proof"),
    );
}

#[test]
fn different_bundle_does_not_rescue_dead_instance() {
    assert!(!desktop_identity_matches_instance(
        b"/Applications/Buzz.app/Contents/MacOS/Buzz\0",
        Some("xyz.block.buzz.app"),
        "xyz.block.buzz.app.dev.proof",
    ));
}
