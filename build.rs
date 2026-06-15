use cc::Build;
use std::{
    env,
    path::{Path, PathBuf},
};

#[cfg(windows)]
const LOCAL_CODEC_ROOT_ENV: &str = "RUSTDESK_WINDOWS_CODEC_ROOT";
#[cfg(target_os = "linux")]
const LOCAL_CODEC_ROOT_ENV: &str = "RUSTDESK_LINUX_CODEC_ROOT";
#[cfg(target_os = "macos")]
const LOCAL_CODEC_ROOT_ENV: &str = "RUSTDESK_MACOS_CODEC_ROOT";
#[cfg(windows)]
const RUSTADMIN_LOCAL_CODEC_ROOT_ENV: &str = "RUSTADMIN_WINDOWS_CODEC_ROOT";
#[cfg(target_os = "linux")]
const RUSTADMIN_LOCAL_CODEC_ROOT_ENV: &str = "RUSTADMIN_LINUX_CODEC_ROOT";
#[cfg(target_os = "macos")]
const RUSTADMIN_LOCAL_CODEC_ROOT_ENV: &str = "RUSTADMIN_MACOS_CODEC_ROOT";
const CMAKE_PREFIX_PATH_ENV: &str = "CMAKE_PREFIX_PATH";
#[cfg(windows)]
const LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTDESK_WINDOWS_CODEC_LINK_MODE";
#[cfg(target_os = "linux")]
const LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTDESK_LINUX_CODEC_LINK_MODE";
#[cfg(target_os = "macos")]
const LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTDESK_MACOS_CODEC_LINK_MODE";
#[cfg(windows)]
const RUSTADMIN_LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTADMIN_WINDOWS_CODEC_LINK_MODE";
#[cfg(target_os = "linux")]
const RUSTADMIN_LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTADMIN_LINUX_CODEC_LINK_MODE";
#[cfg(target_os = "macos")]
const RUSTADMIN_LOCAL_CODEC_LINK_MODE_ENV: &str = "RUSTADMIN_MACOS_CODEC_LINK_MODE";

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

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum LocalLinkKind {
        Static,
        Dynamic,
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum LocalLinkMode {
        Auto,
        Static,
        Dynamic,
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[derive(Clone, Debug)]
    struct LocalLibrary {
        lib_dir: PathBuf,
        lib_path: PathBuf,
        link_name: String,
        kind: LocalLinkKind,
        runtime_paths: Vec<PathBuf>,
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
        if paths.iter().all(|existing| existing != &path) {
            paths.push(path);
        }
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn push_prefix_candidate(paths: &mut Vec<PathBuf>, path: PathBuf) {
        push_unique(paths, path.clone());

        if let Some(parent) = path.parent() {
            if path.file_name().and_then(|name| name.to_str()) == Some("include")
                || path.file_name().and_then(|name| name.to_str()) == Some("lib")
                || path.file_name().and_then(|name| name.to_str()) == Some("lib64")
            {
                push_unique(paths, parent.to_path_buf());
            }
        }

        for ancestor in path.ancestors() {
            if ancestor.join("include").is_dir()
                && (ancestor.join("lib").is_dir() || ancestor.join("lib64").is_dir())
            {
                push_unique(paths, ancestor.to_path_buf());
                break;
            }
        }
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn local_roots() -> Vec<PathBuf> {
        println!("cargo:rerun-if-env-changed={LOCAL_CODEC_ROOT_ENV}");
        println!("cargo:rerun-if-env-changed={RUSTADMIN_LOCAL_CODEC_ROOT_ENV}");
        println!("cargo:rerun-if-env-changed={CMAKE_PREFIX_PATH_ENV}");
        println!("cargo:rerun-if-env-changed={LOCAL_CODEC_LINK_MODE_ENV}");
        println!("cargo:rerun-if-env-changed={RUSTADMIN_LOCAL_CODEC_LINK_MODE_ENV}");

        let mut roots = Vec::new();
        if let Some(path) = env::var_os(LOCAL_CODEC_ROOT_ENV) {
            push_prefix_candidate(&mut roots, PathBuf::from(path));
        }
        if let Some(path) = env::var_os(RUSTADMIN_LOCAL_CODEC_ROOT_ENV) {
            push_prefix_candidate(&mut roots, PathBuf::from(path));
        }
        if let Some(paths) = env::var_os(CMAKE_PREFIX_PATH_ENV) {
            for path in env::split_paths(&paths) {
                push_prefix_candidate(&mut roots, path);
            }
        }

        if let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") {
            let manifest_dir = Path::new(&manifest_dir);
            if let Some(workspace_root) = manifest_dir.parent() {
                #[cfg(windows)]
                let local_dir = "windows-codecs";
                #[cfg(target_os = "linux")]
                let local_dir = "linux-codecs";
                #[cfg(target_os = "macos")]
                let local_dir = "macos-codecs";

                for repo_local_root in [
                    workspace_root
                        .join("rustdesk-client")
                        .join(".local")
                        .join(local_dir),
                    workspace_root.join(".local").join(local_dir),
                    manifest_dir.join(".local").join(local_dir),
                ] {
                    println!("cargo:rerun-if-changed={}", repo_local_root.display());
                    if repo_local_root.exists() {
                        push_prefix_candidate(&mut roots, repo_local_root);
                    }
                }
            }
        }
        roots
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn local_link_mode() -> LocalLinkMode {
        let mode = env::var(LOCAL_CODEC_LINK_MODE_ENV)
            .or_else(|_| env::var(RUSTADMIN_LOCAL_CODEC_LINK_MODE_ENV));
        match mode {
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

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
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

        #[cfg(not(windows))]
        let _ = root;
        #[cfg(windows)]
        let runtime_paths = runtime_dlls_for(root, name);
        #[cfg(not(windows))]
        let runtime_paths = Vec::new();
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn shared_library_ext() -> &'static str {
        #[cfg(target_os = "linux")]
        {
            "so"
        }
        #[cfg(target_os = "macos")]
        {
            "dylib"
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn find_library(root: &Path, names: &[&str], mode: LocalLinkMode) -> Option<LocalLibrary> {
        for lib_dir in [root.join("lib"), root.join("lib64"), root.to_path_buf()] {
            for name in names {
                let normalized = name.trim_start_matches("lib");
                let dynamic_candidates = [
                    lib_dir.join(format!("lib{normalized}.{}", shared_library_ext())),
                    lib_dir.join(format!("{normalized}.{}", shared_library_ext())),
                ];
                let static_candidates = [
                    lib_dir.join(format!("lib{normalized}.a")),
                    lib_dir.join(format!("{normalized}.a")),
                ];

                if mode != LocalLinkMode::Static {
                    for lib_path in dynamic_candidates {
                        if let Some(lib) = local_library_from_candidate(
                            root,
                            &lib_dir,
                            normalized,
                            lib_path,
                            LocalLinkKind::Dynamic,
                            mode,
                        ) {
                            return Some(lib);
                        }
                    }
                }

                if mode != LocalLinkMode::Dynamic {
                    for lib_path in static_candidates {
                        if let Some(lib) = local_library_from_candidate(
                            root,
                            &lib_dir,
                            normalized,
                            lib_path,
                            LocalLinkKind::Static,
                            mode,
                        ) {
                            return Some(lib);
                        }
                    }
                }
            }
        }
        None
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn emit_rerun_if_changed(path: &Path) {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    fn emit_link_search_once(seen: &mut Vec<PathBuf>, lib_dir: &Path) {
        if seen.iter().any(|existing| existing == lib_dir) {
            return;
        }
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        seen.push(lib_dir.to_path_buf());
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
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
    fn emit_optional_static_dependency(
        root: &Path,
        seen_search_dirs: &mut Vec<PathBuf>,
        names: &[&str],
    ) {
        if let Some(lib) = find_library(root, names, LocalLinkMode::Static) {
            emit_link_search_once(seen_search_dirs, &lib.lib_dir);
            emit_local_library(&lib);
        }
    }

    #[cfg(windows)]
    fn emit_static_windows_ffmpeg_private_deps(root: &Path, seen_search_dirs: &mut Vec<PathBuf>) {
        for names in [
            &["aom"][..],
            &["dav1d"][..],
            &["freetype"][..],
            &["harfbuzz"][..],
            &["jxl-static", "jxl"][..],
            &["jxl_cms"][..],
            &["jxl_threads"][..],
            &["hwy"][..],
            &["brotlienc"][..],
            &["brotlidec"][..],
            &["brotlicommon"][..],
            &["lcms2"][..],
            &["libkvazaar", "kvazaar"][..],
            &["libmp3lame-static", "mp3lame"][..],
            &["libmpghip-static", "mpghip"][..],
            &["openjp2"][..],
            &["opus"][..],
            &["vpx"][..],
            &["libwebpmux", "webpmux"][..],
            &["libwebpdemux", "webpdemux"][..],
            &["libwebpdecoder", "webpdecoder"][..],
            &["libwebp", "webp"][..],
            &["libsharpyuv", "sharpyuv"][..],
            &["libx264", "x264"][..],
            &["x265-static", "x265"][..],
            &["libxml2s", "xml2"][..],
            &["iconv"][..],
            &["charset"][..],
            &["lzma"][..],
            &["zstd_static", "zstd"][..],
            &["bz2", "libbz2"][..],
            &["libssl", "ssl"][..],
            &["libcrypto", "crypto"][..],
            &["vpl"][..],
        ] {
            emit_optional_static_dependency(root, seen_search_dirs, names);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const FFMPEG_PC_PACKAGES: &[&str] = &["libavcodec", "libavformat", "libavutil"];
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const FFMPEG_CORE_LIBS: &[&str] = &["avcodec", "avformat", "avutil", "swresample"];

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn prepend_pkg_config_dirs(root: Option<&Path>, command: &mut std::process::Command) {
        let mut pkg_config_paths = Vec::new();
        if let Some(root) = root {
            for dir in [root.join("lib/pkgconfig"), root.join("lib64/pkgconfig")] {
                if dir.is_dir() {
                    push_unique(&mut pkg_config_paths, dir);
                }
            }
        }
        if let Some(paths) = env::var_os("PKG_CONFIG_PATH") {
            for path in env::split_paths(&paths) {
                push_unique(&mut pkg_config_paths, path);
            }
        }
        if let Ok(joined) = env::join_paths(pkg_config_paths) {
            command.env("PKG_CONFIG_PATH", joined);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn run_pkg_config(root: Option<&Path>, args: &[&str]) -> Option<String> {
        let mut command = std::process::Command::new("pkg-config");
        prepend_pkg_config_dirs(root, &mut command);
        command.args(args).args(FFMPEG_PC_PACKAGES);

        let output = command.output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn parse_pkg_config_cflags(output: &str) -> Vec<PathBuf> {
        let mut include_paths = Vec::new();
        let mut iter = output.split_whitespace().peekable();
        while let Some(token) = iter.next() {
            if let Some(path) = token.strip_prefix("-I") {
                if path.is_empty() {
                    if let Some(path) = iter.next() {
                        push_unique(&mut include_paths, PathBuf::from(path));
                    }
                } else {
                    push_unique(&mut include_paths, PathBuf::from(path));
                }
            }
        }
        include_paths
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn emit_library_path(path: &Path, static_link: bool) -> bool {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let Some(parent) = path.parent() else {
            return false;
        };
        let Some(stripped) = file_name
            .strip_prefix("lib")
            .and_then(|name| name.strip_suffix(".a"))
        else {
            return false;
        };
        println!("cargo:rustc-link-search=native={}", parent.display());
        if static_link {
            println!("cargo:rustc-link-lib=static={stripped}");
        } else {
            println!("cargo:rustc-link-lib={stripped}");
        }
        true
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn emit_pkg_config_libs(output: &str, static_ffmpeg: bool) {
        let mut iter = output.split_whitespace().peekable();
        while let Some(token) = iter.next() {
            if let Some(path) = token.strip_prefix("-L") {
                if path.is_empty() {
                    if let Some(path) = iter.next() {
                        println!("cargo:rustc-link-search=native={path}");
                    }
                } else {
                    println!("cargo:rustc-link-search=native={path}");
                }
            } else if let Some(name) = token.strip_prefix("-l") {
                if name.is_empty() {
                    if let Some(name) = iter.next() {
                        emit_pkg_config_lib(name, static_ffmpeg);
                    }
                } else {
                    emit_pkg_config_lib(name, static_ffmpeg);
                }
            } else if let Some(path) = token.strip_prefix("-F") {
                if path.is_empty() {
                    if let Some(path) = iter.next() {
                        println!("cargo:rustc-link-search=framework={path}");
                    }
                } else {
                    println!("cargo:rustc-link-search=framework={path}");
                }
            } else if token == "-framework" {
                if let Some(name) = iter.next() {
                    println!("cargo:rustc-link-lib=framework={name}");
                }
            } else if token == "-pthread" {
                #[cfg(target_os = "linux")]
                println!("cargo:rustc-link-lib=pthread");
            } else if token.ends_with(".a") {
                if !emit_library_path(Path::new(token), true) {
                    println!("cargo:rustc-link-arg={token}");
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn emit_pkg_config_lib(name: &str, static_ffmpeg: bool) {
        let normalized = name.trim_start_matches(':').trim_start_matches("lib");
        let normalized = normalized.strip_suffix(".a").unwrap_or(normalized);
        if static_ffmpeg && FFMPEG_CORE_LIBS.contains(&normalized) {
            println!("cargo:rustc-link-lib=static={normalized}");
        } else {
            println!("cargo:rustc-link-lib={normalized}");
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn link_pkg_config_ffmpeg(
        builder: &mut Build,
        root: Option<&Path>,
        static_ffmpeg: bool,
    ) -> bool {
        let mut lib_args = vec!["--libs"];
        if static_ffmpeg {
            lib_args.push("--static");
        }
        let Some(libs) = run_pkg_config(root, &lib_args) else {
            return false;
        };
        let Some(cflags) = run_pkg_config(root, &["--cflags"]) else {
            return false;
        };

        let mut include_paths = parse_pkg_config_cflags(&cflags);
        if let Some(root) = root {
            let include_dir = root.join("include");
            if include_dir.is_dir() {
                push_unique(&mut include_paths, include_dir);
            }
        }
        for include in include_paths {
            println!("cargo:include={}", include.display());
            emit_rerun_if_changed(&include);
            builder.include(include);
        }
        emit_pkg_config_libs(&libs, static_ffmpeg);
        println!(
            "cargo:warning=Using {} FFmpeg libraries from pkg-config",
            if static_ffmpeg { "static" } else { "dynamic" }
        );
        true
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn link_local_unix(builder: &mut Build) -> bool {
        let link_mode = local_link_mode();

        for root in local_roots() {
            let include_dir = root.join("include");
            let avcodec_header = include_dir.join("libavcodec").join("avcodec.h");
            let avformat_header = include_dir.join("libavformat").join("avformat.h");
            let avutil_header = include_dir.join("libavutil").join("avutil.h");
            if !avcodec_header.exists() || !avformat_header.exists() || !avutil_header.exists() {
                continue;
            }

            let Some(avcodec) = find_library(&root, &["avcodec"], link_mode) else {
                continue;
            };
            let Some(avformat) = find_library(&root, &["avformat"], link_mode) else {
                continue;
            };
            let Some(avutil) = find_library(&root, &["avutil"], link_mode) else {
                continue;
            };

            let ffmpeg_libs = vec![avcodec, avformat, avutil];
            let uses_static_ffmpeg = ffmpeg_libs
                .iter()
                .any(|lib| lib.kind == LocalLinkKind::Static);
            if link_pkg_config_ffmpeg(builder, Some(&root), uses_static_ffmpeg) {
                return true;
            }
            if uses_static_ffmpeg {
                panic!(
                    "Static FFmpeg libraries were found in '{}', but pkg-config metadata for libavcodec/libavformat/libavutil was not usable. Static FFmpeg with hardware-codec deps needs .pc files so private libraries are linked correctly.",
                    root.display()
                );
            }

            let mut link_search_dirs = Vec::new();
            for lib in &ffmpeg_libs {
                emit_link_search_once(&mut link_search_dirs, &lib.lib_dir);
                emit_local_library(lib);
            }
            for header in [&avcodec_header, &avformat_header, &avutil_header] {
                emit_rerun_if_changed(header);
            }
            println!("cargo:include={}", include_dir.display());
            emit_rerun_if_changed(&include_dir);
            builder.include(include_dir);
            println!(
                "cargo:warning=Using dynamic FFmpeg libraries from {}",
                root.display()
            );
            return true;
        }

        false
    }

    #[cfg(windows)]
    fn link_local_windows(builder: &mut Build) -> bool {
        let link_mode = local_link_mode();
        let mut ffmpeg_include = None;
        let mut ffmpeg_root = None;
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
                    ffmpeg_root = Some(root.clone());
                    ffmpeg_libs = Some(vec![avcodec, avutil, avformat, swresample, zlib]);
                }
            }

            if mfx_link.is_none() {
                if let Some(lib) = find_library(&root, &["libmfx", "mfx"], link_mode) {
                    mfx_link = Some(lib);
                }
            }
        }

        let (ffmpeg_include, ffmpeg_root, ffmpeg_libs, mfx_lib) =
            match (ffmpeg_include, ffmpeg_root, ffmpeg_libs, mfx_link) {
                (Some(include), Some(root), Some(libs), Some(mfx)) => (include, root, libs, mfx),
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
            emit_static_windows_ffmpeg_private_deps(&ffmpeg_root, &mut link_search_dirs);
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
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if !link_local_unix(builder)
            && !link_pkg_config_ffmpeg(builder, None, local_link_mode() == LocalLinkMode::Static)
        {
            link_vcpkg(
                builder,
                std::env::var("VCPKG_ROOT")
                    .expect("VCPKG_ROOT is unset and no local/pkg-config FFmpeg was found")
                    .into(),
            );
        }
        #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
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
