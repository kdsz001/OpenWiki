/// Version of yt-dlp to download. Pinned intentionally — bump this when
/// you want to upgrade (after verifying the new release works). Automatic
/// "always fetch latest" would silently break users when yt-dlp ships a
/// change that breaks our invocation.
#[cfg(target_os = "macos")]
const YT_DLP_VERSION: &str = "2026.03.17";

/// Download `yt-dlp_macos` (universal binary) from GitHub releases into
/// `resources/` so it can be bundled into the app via tauri.conf.json.
///
/// Idempotent: if the binary already exists, we leave it alone (avoids
/// network access on every rebuild). Delete the file to force a refresh.
#[cfg(target_os = "macos")]
fn ensure_yt_dlp_bundled(manifest_dir: &std::path::Path) {
    let yt_dlp_path = manifest_dir.join("resources/yt-dlp_macos");

    if yt_dlp_path.exists() {
        return;
    }

    let url = format!(
        "https://github.com/yt-dlp/yt-dlp/releases/download/{}/yt-dlp_macos",
        YT_DLP_VERSION
    );

    println!(
        "cargo:warning=Downloading yt-dlp {} from {} ...",
        YT_DLP_VERSION, url
    );

    let status = std::process::Command::new("curl")
        .args([
            "-L",
            "--fail",
            "--retry",
            "3",
            "-o",
            yt_dlp_path.to_str().unwrap(),
            &url,
        ])
        .status();

    match status {
        Ok(s) if s.success() => {}
        _ => panic!(
            "Failed to download yt-dlp from {}. Check your network, or manually download the file to {} and rerun the build.",
            url,
            yt_dlp_path.display()
        ),
    }

    // Make executable
    let _ = std::process::Command::new("chmod")
        .args(["+x", yt_dlp_path.to_str().unwrap()])
        .status();

    // Ad-hoc codesign so macOS Gatekeeper lets the app launch this binary
    // from inside the bundle. The app itself is ad-hoc signed the same way.
    let sign_status = std::process::Command::new("codesign")
        .args(["--sign", "-", "--force", yt_dlp_path.to_str().unwrap()])
        .status();
    match sign_status {
        Ok(s) if s.success() => {
            println!(
                "cargo:warning=Downloaded and ad-hoc signed yt-dlp -> {}",
                yt_dlp_path.display()
            );
        }
        _ => panic!(
            "Failed to ad-hoc codesign {}. Install the macOS toolchain (it ships with codesign) or sign manually.",
            yt_dlp_path.display()
        ),
    }
}

fn main() {
    // Pre-compile the macOS OCR Swift helper so end users don't need swiftc
    // (i.e. don't need Xcode Command Line Tools) to use OCR at runtime.
    // Also download `yt-dlp_macos` on first build so end users don't need
    // to install yt-dlp via anaconda/brew/pip first.
    // Both binaries are bundled as Tauri resources — see tauri.conf.json.
    #[cfg(target_os = "macos")]
    {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

        ensure_yt_dlp_bundled(&manifest_dir);

        let swift_src = manifest_dir.join("resources/openwiki_ocr.swift");
        let swift_bin = manifest_dir.join("resources/openwiki_ocr_bin");

        println!("cargo:rerun-if-changed={}", swift_src.display());

        if !swift_src.exists() {
            panic!("OCR Swift source not found at {}", swift_src.display());
        }

        // Idempotency check: only invoke swiftc if the binary is missing or
        // older than the source. Rewriting the binary on every build caused
        // a watcher loop in `tauri dev`: write bin → watcher sees change →
        // restart cargo → build.rs runs → write bin → ... (infinite loop).
        let needs_rebuild = match (swift_src.metadata(), swift_bin.metadata()) {
            (Ok(src_meta), Ok(bin_meta)) => match (src_meta.modified(), bin_meta.modified()) {
                (Ok(src_time), Ok(bin_time)) => src_time > bin_time,
                _ => true,
            },
            _ => true,
        };

        if needs_rebuild {
            let status = std::process::Command::new("/usr/bin/swiftc")
                .args([
                    "-O",
                    swift_src.to_str().unwrap(),
                    "-o",
                    swift_bin.to_str().unwrap(),
                ])
                .status();

            match status {
                Ok(s) if s.success() => {
                    println!(
                        "cargo:warning=Pre-compiled OCR Swift binary -> {}",
                        swift_bin.display()
                    );
                }
                Ok(s) => {
                    panic!("swiftc exited with status {} while compiling OCR helper", s);
                }
                Err(e) => {
                    panic!("Failed to invoke swiftc for OCR helper: {}. Is Xcode Command Line Tools installed on the build machine?", e);
                }
            }
        }
    }

    tauri_build::build()
}
