//! Records the commit the app was built from, for the Support report. Falls back to
//! "unknown" when there is no git checkout (for example a source archive).
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The reflog changes on every commit and checkout; the index on every staged edit.
    for path in ["logs/HEAD", "HEAD", "index"] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    let commit = git(&["rev-parse", "--short=12", "HEAD"])
        .filter(|commit| !commit.is_empty())
        .map(|commit| {
            let modified = git(&["status", "--porcelain", "--untracked-files=no"])
                .is_some_and(|s| !s.is_empty());
            if modified {
                format!("{commit}+modified")
            } else {
                commit
            }
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=ELEVENCLOCK_RS_COMMIT={commit}");
}
