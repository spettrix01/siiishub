fn main() {
    // The web server (feature `server` alone) has no Tauri context to build.
    #[cfg(feature = "app")]
    tauri_build::build();

    // Windows: link against the import library generated from libmpv-2.dll and
    // stage the DLL next to the binary. Linux links the system libmpv through
    // libmpv2-sys (`-lmpv`), so there is nothing to do there.
    #[cfg(all(target_os = "windows", feature = "real-mpv"))]
    {
        use std::env;
        use std::path::PathBuf;

        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let bin_dir = manifest.join("binaries");
        if bin_dir.join("mpv.lib").exists() {
            println!("cargo:rustc-link-search=native={}", bin_dir.display());
        } else {
            println!(
                "cargo:warning=mpv.lib not found in {} — generate it with \
                 `dumpbin /exports libmpv-2.dll` + `lib /def:mpv.def \
                 /machine:x64 /out:mpv.lib` (scripts/windows-build.ps1 does it, see docs/BUILDING.md).",
                bin_dir.display()
            );
        }

        let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let target_dir = manifest.join("target").join(&profile);
        let dll_src = bin_dir.join("libmpv-2.dll");
        let dll_dst = target_dir.join("libmpv-2.dll");
        if dll_src.exists() {
            let needs_copy = !dll_dst.exists()
                || std::fs::metadata(&dll_src)
                    .and_then(|s| s.modified())
                    .ok()
                    != std::fs::metadata(&dll_dst)
                        .and_then(|s| s.modified())
                        .ok();
            if needs_copy {
                let _ = std::fs::create_dir_all(&target_dir);
                if let Err(e) = std::fs::copy(&dll_src, &dll_dst) {
                    println!(
                        "cargo:warning=failed to stage libmpv-2.dll → {}: {}",
                        dll_dst.display(),
                        e
                    );
                }
            }
        }
        println!("cargo:rerun-if-changed={}", dll_src.display());
    }
}
