use cc::Build;
use std::{
    env,
    path::{Path, PathBuf},
};

#[cfg(windows)]
const LOCAL_CODEC_ROOT_ENV: &str = "RUSTDESK_WINDOWS_CODEC_ROOT";
#[cfg(windows)]
const CMAKE_PREFIX_PATH_ENV: &str = "CMAKE_PREFIX_PATH";
#[cfg(windows)]
const LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTDESK_WINDOWS_CODEC_LINK_MODE";

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let externals_dir = manifest_dir.join("externals");
    let cpp_dir = manifest_dir.join("cpp");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=deps");
    println!("cargo:rerun-if-changed={}", externals_dir.display());
    println!("cargo:rerun-if-changed={}", cpp_dir.display());
    let mut builder = Build::new();

    build_common(&mut builder);
    ffmpeg::build_ffmpeg(&mut builder);
    #[cfg(all(windows, feature = "vram"))]
    sdk::build_sdk(&mut builder);
    builder.static_crt(true).compile("hwcodec");
}

fn build_common(builder: &mut Build) {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let common_dir = manifest_dir.join("cpp").join("common");
    bindgen::builder()
        .header(common_dir.join("common.h").to_string_lossy().to_string())
        .header(common_dir.join("callback.h").to_string_lossy().to_string())
        .rustified_enum("*")
        .parse_callbacks(Box::new(CommonCallbacks))
        .generate()
        .unwrap()
        .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("common_ffi.rs"))
        .unwrap();

    // system
    #[cfg(windows)]
    {
        for lib in ["d3d11", "dxgi"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    builder.include(&common_dir);

    // platform
    let _platform_path = common_dir.join("platform");
    #[cfg(windows)]
    {
        let win_path = _platform_path.join("win");
        builder.include(&win_path);
        builder.file(win_path.join("win.cpp"));
    }
    #[cfg(target_os = "linux")]
    {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let externals_dir = manifest_dir.join("externals");
        // ffnvcodec
        let ffnvcodec_path = externals_dir
            .join("nv-codec-headers_n12.1.14.0")
            .join("include")
            .join("ffnvcodec");
        builder.include(ffnvcodec_path);

        let linux_path = _platform_path.join("linux");
        builder.include(&linux_path);
        builder.file(linux_path.join("linux.cpp"));
    }
    if target_os == "macos" {
        let macos_path = _platform_path.join("mac");
        builder.include(&macos_path);
        builder.file(macos_path.join("mac.mm"));
    }

    // tool
    builder.files(["log.cpp", "util.cpp"].map(|f| common_dir.join(f)));
}

#[derive(Debug)]
struct CommonCallbacks;
impl bindgen::callbacks::ParseCallbacks for CommonCallbacks {
    fn add_derives(&self, name: &str) -> Vec<String> {
        let names = vec!["DataFormat", "SurfaceFormat", "API"];
        if names.contains(&name) {
            vec!["Serialize", "Deserialize"]
                .drain(..)
                .map(|s| s.to_string())
                .collect()
        } else {
            vec![]
        }
    }
}

mod ffmpeg {
    #[allow(unused_imports)]
    use core::panic;

    use super::*;

    #[cfg(windows)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum LocalLinkKind {
        Static,
        Dynamic,
    }

    #[cfg(windows)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum LocalLinkMode {
        Auto,
        Static,
        Dynamic,
    }

    #[cfg(windows)]
    #[derive(Clone, Debug)]
    struct LocalLibrary {
        lib_dir: PathBuf,
        lib_path: PathBuf,
        link_name: String,
        kind: LocalLinkKind,
        runtime_paths: Vec<PathBuf>,
    }

    #[cfg(windows)]
    fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
        if paths.iter().all(|existing| existing != &path) {
            paths.push(path);
        }
    }

    #[cfg(windows)]
    fn local_roots() -> Vec<PathBuf> {
        println!("cargo:rerun-if-env-changed={LOCAL_CODEC_ROOT_ENV}");
        println!("cargo:rerun-if-env-changed={CMAKE_PREFIX_PATH_ENV}");
        println!("cargo:rerun-if-env-changed={LOCAL_CODEC_LINK_MODE_ENV}");

        let mut roots = Vec::new();
        if let Some(path) = env::var_os(LOCAL_CODEC_ROOT_ENV) {
            push_unique(&mut roots, PathBuf::from(path));
        }
        if let Some(paths) = env::var_os(CMAKE_PREFIX_PATH_ENV) {
            for path in env::split_paths(&paths) {
                push_unique(&mut roots, path);
            }
        }
        roots
    }

    #[cfg(windows)]
    fn local_link_mode() -> LocalLinkMode {
        match env::var(LOCAL_CODEC_LINK_MODE_ENV) {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "" | "auto" => LocalLinkMode::Auto,
                "static" => LocalLinkMode::Static,
                "dynamic" | "dylib" | "shared" => LocalLinkMode::Dynamic,
                other => panic!(
                    "{} must be one of auto, static, or dynamic, got '{}'",
                    LOCAL_CODEC_LINK_MODE_ENV, other
                ),
            },
            Err(_) => LocalLinkMode::Auto,
        }
    }

    #[cfg(windows)]
    fn path_file_name_lower(path: &Path) -> Option<String> {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
    }

    #[cfg(windows)]
    fn runtime_dlls_for(root: &Path, name: &str) -> Vec<PathBuf> {
        let needle = name.trim_start_matches("lib").to_ascii_lowercase();
        let lib_needle = format!("lib{needle}");
        let mut dlls = Vec::new();

        for dir in [
            root.join("bin"),
            root.join("lib"),
            root.join("lib64"),
            root.to_path_buf(),
        ] {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                let Some(file_name) = path_file_name_lower(&path) else {
                    continue;
                };
                if !file_name.ends_with(".dll") {
                    continue;
                }
                if file_name.starts_with(&needle) || file_name.starts_with(&lib_needle) {
                    push_unique(&mut dlls, path);
                }
            }
        }

        dlls
    }

    #[cfg(windows)]
    fn local_library_from_candidate(
        root: &Path,
        lib_dir: &Path,
        name: &str,
        lib_path: PathBuf,
        default_kind: LocalLinkKind,
        mode: LocalLinkMode,
    ) -> Option<LocalLibrary> {
        if !lib_path.exists() {
            return None;
        }

        let runtime_paths = runtime_dlls_for(root, name);
        let kind = match mode {
            LocalLinkMode::Static => LocalLinkKind::Static,
            LocalLinkMode::Dynamic => LocalLinkKind::Dynamic,
            LocalLinkMode::Auto => {
                if default_kind == LocalLinkKind::Dynamic || !runtime_paths.is_empty() {
                    LocalLinkKind::Dynamic
                } else {
                    LocalLinkKind::Static
                }
            }
        };

        Some(LocalLibrary {
            lib_dir: lib_dir.to_path_buf(),
            lib_path,
            link_name: name.to_string(),
            kind,
            runtime_paths,
        })
    }

    #[cfg(windows)]
    fn find_library(root: &Path, names: &[&str], mode: LocalLinkMode) -> Option<LocalLibrary> {
        for lib_dir in [root.join("lib"), root.join("lib64")] {
            for name in names {
                let dynamic_candidates = [
                    lib_dir.join(format!("lib{name}.dll.a")),
                    lib_dir.join(format!("{name}.dll.a")),
                ];
                let static_candidates = [
                    lib_dir.join(format!("{name}.lib")),
                    lib_dir.join(format!("lib{name}.a")),
                    lib_dir.join(format!("{name}.a")),
                ];

                if mode != LocalLinkMode::Static {
                    for lib_path in dynamic_candidates {
                        if let Some(lib) = local_library_from_candidate(
                            root,
                            &lib_dir,
                            name,
                            lib_path,
                            LocalLinkKind::Dynamic,
                            mode,
                        ) {
                            return Some(lib);
                        }
                    }
                }

                for lib_path in static_candidates {
                    if let Some(lib) = local_library_from_candidate(
                        root,
                        &lib_dir,
                        name,
                        lib_path,
                        LocalLinkKind::Static,
                        mode,
                    ) {
                        return Some(lib);
                    }
                }
            }
        }
        None
    }

    #[cfg(windows)]
    fn emit_rerun_if_changed(path: &Path) {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    #[cfg(windows)]
    fn emit_link_search_once(seen: &mut Vec<PathBuf>, lib_dir: &Path) {
        if seen.iter().any(|existing| existing == lib_dir) {
            return;
        }
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        seen.push(lib_dir.to_path_buf());
    }

    #[cfg(windows)]
    fn emit_local_library(lib: &LocalLibrary) {
        emit_rerun_if_changed(&lib.lib_path);
        for runtime_path in &lib.runtime_paths {
            emit_rerun_if_changed(runtime_path);
        }

        match lib.kind {
            LocalLinkKind::Static => {
                println!("cargo:rustc-link-lib=static={}", lib.link_name);
            }
            LocalLinkKind::Dynamic => {
                println!("cargo:rustc-link-lib={}", lib.link_name);
            }
        }
    }

    #[cfg(windows)]
    fn emit_static_windows_ffmpeg_deps() {
        for lib in ["secur32", "ncrypt", "crypt32", "ws2_32"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    #[cfg(windows)]
    fn link_local_windows(builder: &mut Build) -> bool {
        let link_mode = local_link_mode();
        let mut ffmpeg_include = None;
        let mut ffmpeg_libs = None;
        let mut mfx_link = None;

        for root in local_roots() {
            let include_dir = root.join("include");
            let avcodec_header = include_dir.join("libavcodec").join("avcodec.h");
            let avformat_header = include_dir.join("libavformat").join("avformat.h");
            let avutil_header = include_dir.join("libavutil").join("avutil.h");
            if ffmpeg_include.is_none()
                && avcodec_header.exists()
                && avformat_header.exists()
                && avutil_header.exists()
            {
                if let (Some(avcodec), Some(avformat), Some(avutil), Some(swresample), Some(zlib)) = (
                    find_library(&root, &["avcodec"], link_mode),
                    find_library(&root, &["avformat"], link_mode),
                    find_library(&root, &["avutil"], link_mode),
                    find_library(&root, &["swresample"], link_mode),
                    find_library(&root, &["zlib", "zlibstatic", "libz", "z"], link_mode),
                ) {
                    emit_rerun_if_changed(&avcodec_header);
                    emit_rerun_if_changed(&avformat_header);
                    emit_rerun_if_changed(&avutil_header);
                    ffmpeg_include = Some(include_dir);
                    ffmpeg_libs = Some(vec![avcodec, avutil, avformat, swresample, zlib]);
                }
            }

            if mfx_link.is_none() {
                if let Some(lib) = find_library(&root, &["libmfx", "mfx"], link_mode) {
                    mfx_link = Some(lib);
                }
            }
        }

        let (ffmpeg_include, ffmpeg_libs, mfx_lib) = match (ffmpeg_include, ffmpeg_libs, mfx_link) {
            (Some(include), Some(libs), Some(mfx)) => (include, libs, mfx),
            _ => return false,
        };

        let uses_static_ffmpeg = ffmpeg_libs
            .iter()
            .any(|lib| lib.kind == LocalLinkKind::Static);
        let mut link_search_dirs = Vec::new();
        for lib in ffmpeg_libs.iter().chain(std::iter::once(&mfx_lib)) {
            emit_link_search_once(&mut link_search_dirs, &lib.lib_dir);
        }
        for lib in &ffmpeg_libs {
            emit_local_library(lib);
        }
        emit_local_library(&mfx_lib);
        if uses_static_ffmpeg {
            emit_static_windows_ffmpeg_deps();
        }

        let link_summary = match (
            uses_static_ffmpeg,
            ffmpeg_libs
                .iter()
                .any(|lib| lib.kind == LocalLinkKind::Dynamic),
        ) {
            (true, true) => "mixed",
            (true, false) => "static",
            (false, true) => "dynamic",
            _ => return false,
        };
        println!("cargo:warning=Using local Windows FFmpeg libraries ({link_summary} link)");
        println!("cargo:include={}", ffmpeg_include.display());
        emit_rerun_if_changed(&ffmpeg_include);
        builder.include(ffmpeg_include);
        true
    }

    pub fn build_ffmpeg(builder: &mut Build) {
        ffmpeg_ffi();
        #[cfg(windows)]
        if !link_local_windows(builder) {
            link_vcpkg(builder, std::env::var("VCPKG_ROOT").unwrap().into());
        }
        #[cfg(not(windows))]
        link_vcpkg(builder, std::env::var("VCPKG_ROOT").unwrap().into());
        link_os();
        build_ffmpeg_ram(builder);
        #[cfg(feature = "vram")]
        build_ffmpeg_vram(builder);
        build_mux(builder);
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
        if target_os == "macos" || target_os == "ios" {
            builder.flag("-std=c++11");
        }
    }

    fn link_vcpkg(builder: &mut Build, mut path: PathBuf) -> PathBuf {
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
        let mut target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        if target_arch == "x86_64" {
            target_arch = "x64".to_owned();
        } else if target_arch == "x86" {
            target_arch = "x86".to_owned();
        } else if target_arch == "loongarch64" {
            target_arch = "loongarch64".to_owned();
        } else if target_arch == "aarch64" {
            target_arch = "arm64".to_owned();
        } else {
            target_arch = "arm".to_owned();
        }
        let mut target = if target_os == "macos" {
            if target_arch == "x64" {
                "x64-osx".to_owned()
            } else if target_arch == "arm64" {
                "arm64-osx".to_owned()
            } else {
                format!("{}-{}", target_arch, target_os)
            }
        } else if target_os == "windows" {
            "x64-windows-static".to_owned()
        } else {
            format!("{}-{}", target_arch, target_os)
        };
        if target_arch == "x86" {
            target = target.replace("x64", "x86");
        }
        println!("cargo:info={}", target);
        path.push("installed");
        path.push(target);

        println!(
            "{}",
            format!(
                "cargo:rustc-link-search=native={}",
                path.join("lib").to_str().unwrap()
            )
        );
        {
            let mut static_libs = vec!["avcodec", "avutil", "avformat"];
            if target_os == "windows" {
                static_libs.push("libmfx");
            }
            static_libs
                .iter()
                .map(|lib| println!("cargo:rustc-link-lib=static={}", lib))
                .count();
        }

        let include = path.join("include");
        println!("{}", format!("cargo:include={}", include.to_str().unwrap()));
        builder.include(&include);
        include
    }

    fn link_os() {
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
        let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        let dyn_libs: Vec<&str> = if target_os == "windows" {
            [
                "User32", "bcrypt", "ole32", "oleaut32", "advapi32", "uuid", "mf", "mfplat",
                "mfuuid", "strmiids",
            ]
            .to_vec()
        } else if target_os == "linux" {
            let mut v = ["drm", "X11", "stdc++"].to_vec();
            if target_arch == "x86_64" {
                v.push("z");
            }
            v
        } else if target_os == "macos" || target_os == "ios" {
            ["c++", "m"].to_vec()
        } else if target_os == "android" {
            // https://github.com/FFmpeg/FFmpeg/commit/98b5e80fd6980e641199e9ce3bc27100e2df17a4
            // link to mediandk directly since n7.1
            ["z", "m", "android", "atomic", "mediandk"].to_vec()
        } else {
            panic!("unsupported os");
        };
        dyn_libs
            .iter()
            .map(|lib| println!("cargo:rustc-link-lib={}", lib))
            .count();

        if target_os == "macos" || target_os == "ios" {
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
            println!("cargo:rustc-link-lib=framework=CoreVideo");
            println!("cargo:rustc-link-lib=framework=CoreMedia");
            println!("cargo:rustc-link-lib=framework=VideoToolbox");
            println!("cargo:rustc-link-lib=framework=AVFoundation");
        }
    }

    fn ffmpeg_ffi() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ffmpeg_ram_dir = manifest_dir.join("cpp").join("common");
        let ffi_header = ffmpeg_ram_dir
            .join("ffmpeg_ffi.h")
            .to_string_lossy()
            .to_string();
        bindgen::builder()
            .header(ffi_header)
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("ffmpeg_ffi.rs"))
            .unwrap();
    }

    fn build_ffmpeg_ram(builder: &mut Build) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ffmpeg_ram_dir = manifest_dir.join("cpp").join("ffmpeg_ram");
        let ffi_header = ffmpeg_ram_dir
            .join("ffmpeg_ram_ffi.h")
            .to_string_lossy()
            .to_string();
        bindgen::builder()
            .header(ffi_header)
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("ffmpeg_ram_ffi.rs"))
            .unwrap();

        builder.files(
            ["ffmpeg_ram_encode.cpp", "ffmpeg_ram_decode.cpp"].map(|f| ffmpeg_ram_dir.join(f)),
        );
    }

    #[cfg(feature = "vram")]
    fn build_ffmpeg_vram(builder: &mut Build) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ffmpeg_ram_dir = manifest_dir.join("cpp").join("ffmpeg_vram");
        let ffi_header = ffmpeg_ram_dir
            .join("ffmpeg_vram_ffi.h")
            .to_string_lossy()
            .to_string();
        bindgen::builder()
            .header(ffi_header)
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("ffmpeg_vram_ffi.rs"))
            .unwrap();

        builder.files(
            ["ffmpeg_vram_decode.cpp", "ffmpeg_vram_encode.cpp"].map(|f| ffmpeg_ram_dir.join(f)),
        );
    }

    fn build_mux(builder: &mut Build) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mux_dir = manifest_dir.join("cpp").join("mux");
        let mux_header = mux_dir.join("mux_ffi.h").to_string_lossy().to_string();
        bindgen::builder()
            .header(mux_header)
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("mux_ffi.rs"))
            .unwrap();

        builder.files(["mux.cpp"].map(|f| mux_dir.join(f)));
    }
}

#[cfg(all(windows, feature = "vram"))]
mod sdk {
    use super::*;

    pub(crate) fn build_sdk(builder: &mut Build) {
        build_amf(builder);
        build_nv(builder);
        build_mfx(builder);
    }

    fn build_nv(builder: &mut Build) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let externals_dir = manifest_dir.join("externals");
        let common_dir = manifest_dir.join("common");
        let nv_dir = manifest_dir.join("cpp").join("nv");
        println!("cargo:rerun-if-changed=src");
        println!("cargo:rerun-if-changed={}", common_dir.display());
        println!("cargo:rerun-if-changed={}", externals_dir.display());
        bindgen::builder()
            .header(&nv_dir.join("nv_ffi.h").to_string_lossy().to_string())
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("nv_ffi.rs"))
            .unwrap();

        // system
        #[cfg(target_os = "windows")]
        [
            "kernel32", "user32", "gdi32", "winspool", "shell32", "ole32", "oleaut32", "uuid",
            "comdlg32", "advapi32", "d3d11", "dxgi",
        ]
        .map(|lib| println!("cargo:rustc-link-lib={}", lib));
        #[cfg(target_os = "linux")]
        println!("cargo:rustc-link-lib=stdc++");

        // ffnvcodec
        let ffnvcodec_path = externals_dir
            .join("nv-codec-headers_n12.1.14.0")
            .join("include")
            .join("ffnvcodec");
        builder.include(ffnvcodec_path);

        // video codc sdk
        let sdk_path = externals_dir.join("Video_Codec_SDK_12.1.14");
        builder.includes([
            sdk_path.clone(),
            sdk_path.join("Interface"),
            sdk_path.join("Samples").join("Utils"),
            sdk_path.join("Samples").join("NvCodec"),
            sdk_path.join("Samples").join("NvCodec").join("NVEncoder"),
            sdk_path.join("Samples").join("NvCodec").join("NVDecoder"),
        ]);

        for file in vec!["NvEncoder.cpp", "NvEncoderD3D11.cpp"] {
            builder.file(
                sdk_path
                    .join("Samples")
                    .join("NvCodec")
                    .join("NvEncoder")
                    .join(file),
            );
        }
        for file in vec!["NvDecoder.cpp"] {
            builder.file(
                sdk_path
                    .join("Samples")
                    .join("NvCodec")
                    .join("NvDecoder")
                    .join(file),
            );
        }

        // crate
        builder.files(["nv_encode.cpp", "nv_decode.cpp"].map(|f| nv_dir.join(f)));
    }

    fn build_amf(builder: &mut Build) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let externals_dir = manifest_dir.join("externals");
        let amf_dir = manifest_dir.join("cpp").join("amf");
        println!("cargo:rerun-if-changed=src");
        println!("cargo:rerun-if-changed={}", externals_dir.display());
        bindgen::builder()
            .header(amf_dir.join("amf_ffi.h").to_string_lossy().to_string())
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("amf_ffi.rs"))
            .unwrap();

        // system
        #[cfg(windows)]
        println!("cargo:rustc-link-lib=ole32");
        #[cfg(target_os = "linux")]
        println!("cargo:rustc-link-lib=stdc++");

        // amf
        let amf_path = externals_dir.join("AMF_v1.4.35");
        builder.include(format!("{}/amf/public/common", amf_path.display()));
        builder.include(amf_path.join("amf"));

        for f in vec![
            "AMFFactory.cpp",
            "AMFSTL.cpp",
            "Thread.cpp",
            #[cfg(windows)]
            "Windows/ThreadWindows.cpp",
            #[cfg(target_os = "linux")]
            "Linux/ThreadLinux.cpp",
            "TraceAdapter.cpp",
        ] {
            builder.file(format!("{}/amf/public/common/{}", amf_path.display(), f));
        }

        // crate
        builder.files(["amf_encode.cpp", "amf_decode.cpp"].map(|f| amf_dir.join(f)));
    }

    fn build_mfx(builder: &mut Build) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let externals_dir = manifest_dir.join("externals");
        let mfx_dir = manifest_dir.join("cpp").join("mfx");
        println!("cargo:rerun-if-changed=src");
        println!("cargo:rerun-if-changed={}", externals_dir.display());
        bindgen::builder()
            .header(&mfx_dir.join("mfx_ffi.h").to_string_lossy().to_string())
            .rustified_enum("*")
            .generate()
            .unwrap()
            .write_to_file(Path::new(&env::var_os("OUT_DIR").unwrap()).join("mfx_ffi.rs"))
            .unwrap();

        // MediaSDK
        let sdk_path = externals_dir.join("MediaSDK_22.5.4");

        // mfx_dispatch
        let mfx_path = sdk_path.join("api").join("mfx_dispatch");
        // include headers and reuse static lib
        builder.include(mfx_path.join("windows").join("include"));

        let sample_path = sdk_path.join("samples").join("sample_common");
        builder
            .includes([
                sdk_path.join("api").join("include"),
                sample_path.join("include"),
            ])
            .files(
                [
                    "sample_utils.cpp",
                    "base_allocator.cpp",
                    "d3d11_allocator.cpp",
                    "avc_bitstream.cpp",
                    "avc_spl.cpp",
                    "avc_nal_spl.cpp",
                ]
                .map(|f| sample_path.join("src").join(f)),
            )
            .files(
                [
                    "time.cpp",
                    "atomic.cpp",
                    "shared_object.cpp",
                    "thread_windows.cpp",
                ]
                .map(|f| sample_path.join("src").join("vm").join(f)),
            );

        // link
        [
            "kernel32", "user32", "gdi32", "winspool", "shell32", "ole32", "oleaut32", "uuid",
            "comdlg32", "advapi32", "d3d11", "dxgi",
        ]
        .map(|lib| println!("cargo:rustc-link-lib={}", lib));

        builder
            .files(["mfx_encode.cpp", "mfx_decode.cpp"].map(|f| mfx_dir.join(f)))
            .define("NOMINMAX", None)
            .define("MFX_DEPRECATED_OFF", None)
            .define("MFX_D3D11_SUPPORT", None);
    }
}
