use std::process::Command;

fn main() {
    // Short git commit hash (or "unknown" outside a git checkout).
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Mark the tree dirty if there are uncommitted changes.
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    let git_desc = if dirty {
        format!("{}-dirty", git_hash)
    } else {
        git_hash
    };

    // Build timestamp (UTC, second precision).
    let build_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    println!("cargo:rustc-env=SJBIS_GIT_HASH={}", git_desc);
    println!("cargo:rustc-env=SJBIS_BUILD_TIME={}", build_time);

    // Rebuild when HEAD or the index changes so the hash stays fresh.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
