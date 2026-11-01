fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Host test/example executables also link ScreenCaptureKit's Swift bridge.
    // Dependency build-script link arguments do not set their runtime rpath.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        build_camera_bridge();
    }
}

fn build_camera_bridge() {
    use std::{env, path::PathBuf, process::Command};
    println!("cargo:rerun-if-changed=native/macos_camera.swift");
    let out=PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch=match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64"=>"arm64", "x86_64"=>"x86_64", other=>panic!("Unsupported camera target: {other}"),
    };
    let object=out.join("macos_camera.o");
    let status=Command::new("xcrun").args(["swiftc","-emit-object","-parse-as-library","-O",
        "-swift-version","5","-module-name","WeaveloCamera","-target",&format!("{arch}-apple-macosx13.0"),
        "native/macos_camera.swift","-o"]).arg(&object).status().expect("Run the existing macOS Swift SDK");
    assert!(status.success(),"Native camera bridge compilation failed");
    let archive=out.join("libWeaveloCamera.a");
    let status=Command::new("xcrun").args(["libtool","-static","-o"]).arg(archive).arg(object)
        .status().expect("Archive native camera bridge");
    assert!(status.success(),"Native camera bridge archive failed");
    println!("cargo:rustc-link-search=native={}",out.display());
    println!("cargo:rustc-link-lib=static=WeaveloCamera");
    for framework in ["AVFoundation","CoreMedia","CoreVideo","Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
