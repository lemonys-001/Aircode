use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=vendor/sqlite3/sqlite3.c");
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/icon.ico");
    println!("cargo:rerun-if-changed=resources/app.manifest");

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

        // Optional resource embedding (.ico + PerMonitorV2 DPI manifest + VERSIONINFO)
        if let Ok(res_file) = env::var("PREBUILT_WIN32_RES") {
            println!("cargo:rustc-link-arg-bin=AI-Coding-Agent={}", res_file);
        } else if target_env == "msvc" {
            let res_out = out_dir.join("app.res");
            if let Ok(st) = Command::new("rc.exe")
                .args(["/nologo", "/fo"])
                .arg(&res_out)
                .arg("resources/app.rc")
                .status()
            {
                if st.success() {
                    println!("cargo:rustc-link-arg-bin=AI-Coding-Agent={}", res_out.display());
                }
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
