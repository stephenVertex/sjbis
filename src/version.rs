/// Crate version from Cargo.toml.
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git hash (with "-dirty" suffix if the tree had uncommitted changes).
pub const GIT_HASH: &str = env!("SJBIS_GIT_HASH");

/// Build time as a unix timestamp (seconds), as a string.
pub const BUILD_TIME: &str = env!("SJBIS_BUILD_TIME");

/// Human-readable version string, e.g. "0.1.0+a1b2c3d".
pub fn full() -> String {
    format!("{}+{}", PKG_VERSION, GIT_HASH)
}

/// Build time as an RFC3339 UTC string (best effort).
pub fn build_time_rfc3339() -> String {
    let secs: i64 = BUILD_TIME.parse().unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_string())
}
