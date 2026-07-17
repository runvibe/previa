use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo_root = manifest_dir.parent().expect("workspace root");
    let app_dir = repo_root.join("app");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    let embedded_dir = out_dir.join("app-dist");

    println!("cargo:rerun-if-env-changed=PREVIA_MAIN_SKIP_APP_BUILD");
    println!(
        "cargo:rerun-if-changed={}",
        app_dir.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        app_dir.join("package-lock.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        app_dir.join("index.html").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        app_dir.join("vite.config.ts").display()
    );
    println!("cargo:rerun-if-changed={}", app_dir.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        app_dir.join("public").display()
    );

    recreate_dir(&embedded_dir);

    if !app_dir.join("package.json").exists() {
        return;
    }

    let skip_build = env::var("PREVIA_MAIN_SKIP_APP_BUILD")
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);

    if !skip_build {
        run(Command::new("npm").arg("ci").current_dir(&app_dir));
        run(Command::new("npm")
            .arg("run")
            .arg("build")
            .current_dir(&app_dir));
    }

    let dist_dir = app_dir.join("dist");
    if dist_dir.exists() {
        copy_dir(&dist_dir, &embedded_dir);
    }
}

fn run(command: &mut Command) {
    let status = command.status().expect("failed to run app build command");
    if !status.success() {
        panic!("app build command failed with status {status}");
    }
}

fn recreate_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("failed to remove embedded app dir");
    }
    fs::create_dir_all(path).expect("failed to create embedded app dir");
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("failed to create target dir");
    for entry in fs::read_dir(source).expect("failed to read source dir") {
        let entry = entry.expect("failed to read source entry");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type().expect("failed to read source file type");
        if file_type.is_dir() {
            copy_dir(&source_path, &target_path);
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).expect("failed to copy app asset");
        }
    }
}
