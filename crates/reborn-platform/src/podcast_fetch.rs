//! Podcast feeds and episode audio over HTTP(S) through the system curl, so
//! TLS and certificates come from the platform (Y2Linux ships curl and the
//! CA bundle). Errors are worded for the UI.
use std::{
    path::Path,
    process::{Command, Stdio},
};

/// Feeds larger than this are refused (long-running shows reach a few MB).
pub const MAX_FEED_BYTES: u64 = 8 * 1024 * 1024;
/// Episodes larger than this are refused.
pub const MAX_EPISODE_BYTES: u64 = 1024 * 1024 * 1024;
const USER_AGENT: &str = "Reborn music player for Innioasis Y2 (Y2Linux)";

/// `plain_ok` allows plain HTTP, for the address and its redirects; only
/// feeds listed as `http://` and their audio need it.
fn curl(url: &str, max_bytes: u64, plain_ok: bool) -> Result<Command, String> {
    if !reborn_core::podcasts::is_web_address(url) {
        return Err("Not a web address".into());
    }
    let protocols = if plain_ok { "=http,https" } else { "=https" };
    let mut command = Command::new("curl");
    command
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-redirs",
            "10",
            "--proto",
            protocols,
            "--proto-redir",
            protocols,
            "--user-agent",
            USER_AGENT,
            "--connect-timeout",
            "15",
            "--max-filesize",
            &max_bytes.to_string(),
        ])
        .stdin(Stdio::null());
    Ok(command)
}

/// The feed at `url`.
pub fn feed(url: &str, plain_ok: bool) -> Result<Vec<u8>, String> {
    let out = curl(url, MAX_FEED_BYTES, plain_ok)?
        .args(["--max-time", "60", url])
        .output()
        .map_err(|e| format!("curl unavailable: {e}"))?;
    if !out.status.success() {
        return Err(failure(out.status.code()));
    }
    Ok(out.stdout)
}

/// Downloads `url` into `dest` (replacing it). A transfer that stalls for a
/// minute is given up.
pub fn download(url: &str, dest: &Path, plain_ok: bool) -> Result<(), String> {
    let status = curl(url, MAX_EPISODE_BYTES, plain_ok)?
        .args(["--speed-limit", "1000", "--speed-time", "60", "--output"])
        .arg(dest)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("curl unavailable: {e}"))?;
    if !status.success() {
        return Err(failure(status.code()));
    }
    Ok(())
}

fn failure(code: Option<i32>) -> String {
    match code {
        Some(6) => "No internet connection".into(),
        Some(7) => "Couldn't reach the podcast's server".into(),
        Some(22) => "The podcast's server refused the request".into(),
        Some(23) => "Couldn't write to the SD card".into(),
        Some(28) => "The podcast's server timed out".into(),
        Some(35 | 60) => "Secure connection failed (check the clock)".into(),
        Some(63) => "Too large to download".into(),
        code => format!("Download failed (curl {code:?})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_reach_curl() {
        assert!(feed("--output=/etc/x", true).is_err());
        assert!(download("file:///etc/passwd", Path::new("/tmp/x"), true).is_err());
        assert_eq!(failure(Some(6)), "No internet connection");
    }
}
