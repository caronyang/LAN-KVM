use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=resources/lankvm-service.rc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64") {
        println!("cargo:warning=version resources skipped for non-x86_64 target");
        return;
    }

    let Some(rc_exe) = find_rc() else {
        println!("cargo:warning=rc.exe not found; building without version resources");
        return;
    };

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out_dir.join("lankvm-service.res");
    let rc = PathBuf::from("resources/lankvm-service.rc");

    let status = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&rc)
        .status();

    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-link-arg={}", res.display());
        }
        _ => {
            println!("cargo:warning=rc.exe failed; building without version resources");
        }
    }
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(output) = Command::new("where").arg("rc.exe").output() {
        if output.status.success() {
            if let Some(line) = String::from_utf8_lossy(&output.stdout).lines().next() {
                let path = PathBuf::from(line.trim());
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    let candidates = [
        r"C:\Program Files (x86)\Windows Kits\10\bin\x64\rc.exe",
        r"C:\Program Files (x86)\Windows Kits\8.1\bin\x64\rc.exe",
        r"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\rc.exe",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}
