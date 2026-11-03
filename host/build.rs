fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var_os("CARGO_FEATURE_NATIVE_LINUX_VIDEO").is_some()
    {
        let headers = std::path::PathBuf::from(std::env::var_os("WEAVELO_VERIFIED_FFMPEG_INCLUDE")
            .expect("native-linux-video uses the existing verified FFmpeg 8.x SDK"));
        cc::Build::new().file("native/linux_encode.c").include(headers).std("c11")
            .define("_POSIX_C_SOURCE", "200809L").opt_level(2).warnings(true)
            .extra_warnings(true).warnings_into_errors(true).pic(true).compile("weavelo_linux_encode");
        for library in ["libavcodec.so.62", "libavutil.so.60", "libavfilter.so.11", "libva.so.2"] {
            println!("cargo:rustc-link-lib=dylib:+verbatim={library}");
        }
        println!("cargo:rerun-if-changed=native/linux_encode.c");
    }
    println!("cargo:rerun-if-env-changed=WEAVELO_VERIFIED_FFMPEG_INCLUDE");
    // Host test/example executables also link ScreenCaptureKit's Swift bridge.
    // Dependency build-script link arguments do not set their runtime rpath.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        build_camera_bridge();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=native/windows_camera.cpp");
        let mut build=cc::Build::new();build.cpp(true).std("c++17").file("native/windows_camera.cpp")
            .define("NOMINMAX",None).define("WIN32_LEAN_AND_MEAN",None);
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref()==Ok("msvc"){build.flag("/EHsc");}
        build.compile("weavelo_camera");
        for library in ["mfplat","mf","mfreadwrite","mfuuid","d3d11","dxgi","ole32","oleaut32","cfgmgr32"] {
            println!("cargo:rustc-link-lib={library}");
        }
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
