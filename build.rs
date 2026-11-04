use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SKIP: &str = "CLAUDE_REVIEW_SKIP_UI";

const STUB: &str = "<!doctype html><meta charset=utf-8><title>claude-review</title>\
<p>This binary was built with CLAUDE_REVIEW_SKIP_UI set, so it has no interface. \
Rebuild without it.";

fn watched(web: &Path) {
    for path in [
        "src",
        "dist",
        "index.html",
        "package.json",
        "yarn.lock",
        "vite.config.ts",
        "tsconfig.json",
    ] {
        // Cargo reruns every build for a missing path, and a packaged crate
        // ships only the prebuilt bundle.
        let path = web.join(path);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    println!("cargo:rerun-if-env-changed={SKIP}");
}

fn write_stub(out: &Path) {
    fs::create_dir_all(out).expect("create bundle directory");
    fs::write(out.join("index.html"), STUB).expect("write stub shell");
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create bundle directory");
    for entry in fs::read_dir(from).expect("read prebuilt bundle") {
        let entry = entry.expect("read prebuilt bundle");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("stat prebuilt bundle").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy prebuilt bundle");
        }
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let web = manifest.join("web");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("web");

    watched(&web);

    if env::var_os(SKIP).is_some() {
        println!("cargo:warning={SKIP} is set: building without the browser interface");
        write_stub(&out);
        return;
    }

    let vite = web
        .join("node_modules")
        .join(".bin")
        .join(if cfg!(windows) { "vite.cmd" } else { "vite" });

    // A published crate has no node_modules but ships web/dist, built before
    // packaging. A checkout with node_modules always rebuilds, so a stale dist
    // never wins.
    let prebuilt = web.join("dist");
    if !vite.exists() && prebuilt.join("index.html").is_file() {
        if out.exists() {
            fs::remove_dir_all(&out).expect("clear bundle directory");
        }
        copy_dir(&prebuilt, &out);
        return;
    }

    if !vite.exists() {
        panic!(
            "the browser interface cannot be built: {} is missing.\n\
             Run `yarn --cwd web install`, or set {SKIP}=1 to build without it.",
            vite.display()
        );
    }

    let status = Command::new(&vite)
        .current_dir(&web)
        .arg("build")
        .arg("--outDir")
        .arg(&out)
        .arg("--emptyOutDir")
        .status();

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("`vite build` failed with {status}"),
        Err(err) => panic!(
            "could not run {}: {err}\n\
             Set {SKIP}=1 to build without the interface.",
            vite.display()
        ),
    }

    if !out.join("index.html").is_file() {
        panic!("`vite build` produced no index.html in {}", out.display());
    }
}
