use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe(target_arch: &str) -> Option<PathBuf> {
    let sdk_arch = match target_arch {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => "x64",
    };

    if let Ok(output) = Command::new("where.exe").arg("rc.exe").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = stdout.lines().next() {
                let p = PathBuf::from(first.trim());
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }

    let kits_bin = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let Ok(entries) = std::fs::read_dir(kits_bin) else {
        return None;
    };

    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(sdk_arch).join("rc.exe"))
        .filter(|p| p.exists())
        .collect();
    candidates.sort();
    candidates.pop()
}

fn main() {
    println!("cargo:rerun-if-changed=setup.rc");
    println!("cargo:rerun-if-changed=../../assets/knu_diagnosis.ico");
    println!("cargo:rerun-if-changed=../../target/release/knu_diagnosis.exe");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".to_string());

    let rc_file = manifest_dir.join("setup.rc");
    let res_file = out_dir.join("knu_diagnosis_setup.res");

    let rc_exe = find_rc_exe(&target_arch)
        .unwrap_or_else(|| panic!("Could not locate Windows SDK rc.exe for {target_arch}"));

    let status = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_file)
        .arg(&rc_file)
        .status()
        .unwrap_or_else(|err| panic!("Failed to execute {}: {err}", rc_exe.display()));

    if !status.success() {
        panic!("rc.exe failed with status {status}");
    }

    println!("cargo:rustc-link-arg={}", res_file.display());
}
