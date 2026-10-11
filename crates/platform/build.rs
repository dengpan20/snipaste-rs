fn main() {
    println!("cargo:rerun-if-changed=src/macos/native.m");
    println!("cargo:rerun-if-env-changed=SNIPASTE_MACOS_CHECK_ONLY");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if !cfg!(target_os = "macos") {
        // Only Rust type checking is possible without Apple's SDK. Never use this for builds.
        assert!(
            std::env::var("SNIPASTE_MACOS_CHECK_ONLY").as_deref() == Ok("1"),
            "macOS builds require a Mac and the Xcode Command Line Tools"
        );
        println!("cargo:warning=Rust-only macOS check: native AppKit/ScreenCaptureKit bridge requires Apple's SDK");
        return;
    }
    cc::Build::new()
        .file("src/macos/native.m")
        .flag("-fobjc-arc")
        .flag("-fblocks")
        .flag("-mmacosx-version-min=14.0")
        .flag("-Wall")
        .flag("-Wextra")
        .flag("-Werror")
        .compile("snipaste_macos");
    for framework in [
        "AppKit",
        "Foundation",
        "CoreGraphics",
        "CoreText",
        "ScreenCaptureKit",
        "UniformTypeIdentifiers",
        "QuartzCore",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
