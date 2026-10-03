use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/dictation/permission.m");
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let object = out.join("voice-permission.o");
        assert!(
            Command::new("clang")
                .args(["-fobjc-arc", "-c", "src/dictation/permission.m", "-o"])
                .arg(&object)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("ar")
                .arg("crus")
                .arg(out.join("libvoice-permission.a"))
                .arg(object)
                .status()
                .unwrap()
                .success()
        );
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=voice-permission");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
    }

    println!("cargo:rerun-if-changed=src/browser/linux/helper.c");
    println!("cargo:rerun-if-changed=src/browser/linux/helper_wpe.c");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("zeron-webkit");
    let pkg_config = |modules: &[&str]| {
        Command::new("pkg-config")
            .args(["--cflags", "--libs"])
            .args(modules)
            .output()
            .expect("pkg-config is required to build the Linux browser helper")
    };
    // WPE WebKit lets the helper act as the display and set the frame rate;
    // WebKitGTK renders offscreen at a fixed 60 Hz. Prefer WPE where its
    // development files exist and fall back otherwise.
    let wpe = pkg_config(&[
        "wpe-webkit-2.0",
        "wpe-platform-2.0",
        "json-glib-1.0",
        "xkbcommon",
    ]);
    let (engine, source, flags) = if wpe.status.success() {
        ("WPE WebKit", "src/browser/linux/helper_wpe.c", wpe)
    } else {
        let gtk = pkg_config(&["webkit2gtk-4.1", "json-glib-1.0"]);
        assert!(
            gtk.status.success(),
            "The Linux browser requires WPE WebKit (wpe-webkit-2.0, wpe-platform-2.0, \
             xkbcommon) or WebKitGTK 4.1 (webkit2gtk-4.1), plus JSON-GLib (json-glib-1.0), \
             with development files discoverable by pkg-config. \
             See docs/reference/linux-browser.md for distribution-specific installation commands.\n{}",
            String::from_utf8_lossy(&gtk.stderr)
        );
        ("WebKitGTK 4.1", "src/browser/linux/helper.c", gtk)
    };
    println!("cargo:rustc-env=ZERON_BROWSER_ENGINE={engine}");
    let status = Command::new(env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args([
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Wno-unused-parameter",
            source,
            "-o",
        ])
        .arg(&output)
        .args(String::from_utf8(flags.stdout).unwrap().split_whitespace())
        .status()
        .expect("C compiler is required to build the Linux browser helper");
    assert!(status.success(), "Linux browser helper compilation failed");
}
