//! Stamp the build's base revision; parser source and dependency hashes identify dirty builds.
use std::process::Command;
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn main() {
    for path in ["src", "Cargo.lock", "build.rs", ".git"] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=S106_SOFTWARE_REVISION");
    if let Some(revision) = git(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=S106_BUILD_REVISION={revision}");
    }
    if let Some(status) = git(&["status", "--porcelain"]) {
        println!("cargo:rustc-env=S106_BUILD_DIRTY={}", !status.is_empty());
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &branch])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    // Managed worktrees keep HEAD and the index outside the checkout.
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
