use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../dist/AI-Coding-Agent.exe");
    println!("cargo:rerun-if-changed=../resources/setup.rc");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" {
        if let Ok(res_file) = env::var("PREBUILT_SETUP_RES") {
            println!(
                "cargo:rustc-link-arg-bin=AI-Coding-Agent-Setup={}",
                res_file
            );
        } else if target_env == "msvc" {
            let out_dir = env::var("OUT_DIR").expect("OUT_DIR");
            let res_out = format!("{}/setup.res", out_dir);
            if let Ok(st) = Command::new("rc.exe")
                .args(["/nologo", "/fo", &res_out, "../resources/setup.rc"])
                .status()
            {
                if st.success() {
                    println!("cargo:rustc-link-arg-bin=AI-Coding-Agent-Setup={}", res_out);
                }
            }
        }

        for lib in [
            "user32", "gdi32", "shell32", "ole32", "advapi32", "dwmapi", "uuid",
        ] {
            println!("cargo:rustc-link-lib=dylib={}", lib);
        }
    }
}
