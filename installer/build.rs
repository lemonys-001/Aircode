use std::env;
use std::fs;
use std::path::Path;

#[path = "../src/winres.rs"]
mod winres;

fn main() {
    println!("cargo:rerun-if-changed=../dist/AI-Coding-Agent.exe");
    println!("cargo:rerun-if-changed=../resources/setup.rc");
    println!("cargo:rerun-if-changed=../resources/icon.ico");
    println!("cargo:rerun-if-changed=../resources/app.manifest");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "windows" {
        // Same pure-Rust .res generation as the main crate: no rc.exe needed,
        // so the installer ships its icon + version info on every toolchain.
        let machine: u16 = if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") {
            0xAA64
        } else {
            0x8664
        };
        if let Ok(res_file) = env::var("PREBUILT_SETUP_RES") {
            println!(
                "cargo:rustc-link-arg-bin=AI-Coding-Agent-Setup={}",
                res_file
            );
        } else {
            let version = winres::VersionInfo::from_crate_version(
                &env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string()),
                "AI-Coding-Agent-Setup",
                "AI Coding Agent Setup Wizard — Pure-Rust Windows x64 Installer",
            );
            match winres::generate_from_paths(
                machine,
                Path::new("../resources/icon.ico"),
                Some(Path::new("../resources/app.manifest")),
                &version,
            ) {
                Ok(bytes) => {
                    let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
                    let res_out = Path::new(&out_dir).join("setup.res");
                    if fs::write(&res_out, &bytes).is_ok() {
                        println!(
                            "cargo:rustc-link-arg-bin=AI-Coding-Agent-Setup={}",
                            res_out.display()
                        );
                        println!("cargo:warning=embedded win32 resources (icon+version+manifest)");
                    }
                }
                Err(e) => println!(
                    "cargo:warning=setup win32 resource generation failed ({}); icon missing",
                    e
                ),
            }
        }

        for lib in [
            "user32", "gdi32", "shell32", "ole32", "advapi32", "dwmapi", "uuid",
        ] {
            println!("cargo:rustc-link-lib=dylib={}", lib);
        }
    }
}
