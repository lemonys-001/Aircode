use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[path = "src/winres.rs"]
mod winres;

fn main() {
    println!("cargo:rerun-if-changed=vendor/sqlite3/sqlite3.c");
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/icon.ico");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    // The vendored SQLite amalgamation is compiled by $CC, so switching to a
    // cross compiler (or a different one) MUST invalidate the cached object.
    // Without this, a host-built sqlite3.o gets reused for a foreign target and
    // the link fails with "sqlite3.o is incompatible with <arch>".
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=AR");
    println!("cargo:rerun-if-env-changed=PREBUILT_SQLITE3_LIB");
    println!("cargo:rerun-if-env-changed=PREBUILT_WIN32_RES");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    if target_os == "windows" {
        let lib_name = if target_env == "msvc" {
            "sqlite3.lib"
        } else {
            "libsqlite3.a"
        };
        let dst_lib = out_dir.join(lib_name);

        if let Ok(prebuilt) = env::var("PREBUILT_SQLITE3_LIB") {
            fs::copy(&prebuilt, &dst_lib).expect("Failed copying PREBUILT_SQLITE3_LIB");
        } else if target_env == "msvc" {
            let obj = out_dir.join("sqlite3.obj");
            let status = Command::new("cl.exe")
                .args([
                    "/nologo",
                    "/O2",
                    "/c",
                    "/DSQLITE_THREADSAFE=1",
                    "/DSQLITE_ENABLE_FTS5",
                    "/DSQLITE_ENABLE_JSON1",
                    "vendor/sqlite3/sqlite3.c",
                ])
                .arg(format!("/Fo{}", obj.display()))
                .status()
                .expect("Failed to invoke cl.exe for sqlite3.c");
            assert!(status.success(), "cl.exe failed compiling sqlite3.c");

            let ar_status = Command::new("lib.exe")
                .arg("/nologo")
                .arg(format!("/OUT:{}", dst_lib.display()))
                .arg(&obj)
                .status()
                .expect("Failed to invoke lib.exe");
            assert!(ar_status.success(), "lib.exe failed archiving sqlite3.lib");
        } else {
            let cc = env::var("CC").unwrap_or_else(|_| "x86_64-w64-mingw32-gcc".to_string());
            let ar = env::var("AR").unwrap_or_else(|_| "x86_64-w64-mingw32-ar".to_string());
            let obj = out_dir.join("sqlite3.o");
            let status = Command::new(&cc)
                .args([
                    "-O2",
                    "-DSQLITE_THREADSAFE=1",
                    "-DSQLITE_ENABLE_FTS5",
                    "-DSQLITE_ENABLE_JSON1",
                    "-c",
                    "vendor/sqlite3/sqlite3.c",
                    "-o",
                ])
                .arg(&obj)
                .status()
                .expect("Failed compiling sqlite3.c");
            assert!(status.success(), "Compiler failed on sqlite3.c");

            let ar_status = Command::new(&ar)
                .args(["rcs"])
                .arg(&dst_lib)
                .arg(&obj)
                .status()
                .expect("Failed archiving libsqlite3.a");
            assert!(ar_status.success(), "Archiver failed on libsqlite3.a");
        }

        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=sqlite3");

        // Windows resource embedding (.ico + PerMonitorV2 DPI manifest + VERSIONINFO).
        //
        // rc.exe / windres / llvm-rc are absent from most cross toolchains (zig
        // ships no `rc` subcommand), which silently produced icon-less binaries.
        // We therefore generate the COFF `.res` ourselves in pure Rust and hand
        // it straight to the linker. `PREBUILT_WIN32_RES` still wins if set.
        let machine: u16 = if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") {
            0xAA64
        } else {
            0x8664
        };
        if let Ok(res_file) = env::var("PREBUILT_WIN32_RES") {
            println!("cargo:rustc-link-arg-bin=aircode={}", res_file);
        } else {
            let version = winres::VersionInfo::from_crate_version(
                &env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string()),
                "aircode",
                "AirCode — Pure-Rust Desktop",
            );
            match winres::generate_from_paths(
                machine,
                Path::new("resources/icon.ico"),
                Some(Path::new("resources/app.manifest")),
                &version,
            ) {
                Ok(bytes) => {
                    let res_out = out_dir.join("app.res");
                    if fs::write(&res_out, &bytes).is_ok() {
                        println!(
                            "cargo:rustc-link-arg-bin=aircode={}",
                            res_out.display()
                        );
                        println!("cargo:warning=embedded win32 resources (icon+version+manifest)");
                    } else {
                        println!("cargo:warning=could not write app.res; icon will be missing");
                    }
                }
                Err(e) => println!(
                    "cargo:warning=win32 resource generation failed ({}); icon will be missing",
                    e
                ),
            }
        }

        for lib in [
            "user32", "gdi32", "shell32", "ole32", "advapi32", "dwmapi", "winhttp", "comdlg32",
            "imm32", "uuid",
        ] {
            println!("cargo:rustc-link-lib=dylib={}", lib);
        }
    } else {
        // Linux / Unix build
        let dst_lib = out_dir.join("libsqlite3.a");
        if let Ok(prebuilt) = env::var("PREBUILT_SQLITE3_LIB") {
            fs::copy(&prebuilt, &dst_lib).expect("Failed copying PREBUILT_SQLITE3_LIB");
        } else {
            let obj = out_dir.join("sqlite3.o");
            let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
            let status = Command::new(&cc)
                .args([
                    "-O2",
                    "-DSQLITE_THREADSAFE=1",
                    "-DSQLITE_ENABLE_FTS5",
                    "-DSQLITE_ENABLE_JSON1",
                    "-c",
                    "vendor/sqlite3/sqlite3.c",
                    "-o",
                ])
                .arg(&obj)
                .status()
                .expect("Failed compiling sqlite3.c with cc");
            assert!(status.success(), "cc failed on sqlite3.c");

            let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());
            let ar_status = Command::new(&ar)
                .args(["rcs"])
                .arg(&dst_lib)
                .arg(&obj)
                .status()
                .expect("Failed archiving libsqlite3.a");
            assert!(ar_status.success(), "ar failed on libsqlite3.a");
        }

        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=sqlite3");
        println!("cargo:rustc-link-lib=dylib=pthread");
        println!("cargo:rustc-link-lib=dylib=dl");
        println!("cargo:rustc-link-lib=dylib=m");
    }
}
