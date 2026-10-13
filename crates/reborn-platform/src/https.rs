//! Small HTTPS GETs through the system curl, so TLS and certificates come
//! from the platform. Bodies are bounded; errors are worded for the UI.
use std::process::{Command, Stdio};

const USER_AGENT: &str = "Reborn music player for Innioasis Y2 (Y2Linux)";

pub(crate) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// One GET whose body may be up to `max_body` bytes; redirects are followed
/// but only to HTTPS.
pub(crate) fn get(url: &str, max_body: usize) -> Result<Response, String> {
    let out = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--location",
            "--proto-redir",
            "=https",
            "--user-agent",
            USER_AGENT,
            "--connect-timeout",
            "8",
            "--max-time",
            "15",
            "--max-filesize",
            &max_body.to_string(),
            "--write-out",
            "\n%{http_code}",
            url,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("curl unavailable: {e}"))?;
    if !out.status.success() {
        return Err(match out.status.code() {
            Some(1) => "Only secure (https) addresses are used".into(),
            Some(6) => "No internet connection".into(),
            Some(7) => "Couldn't reach the service".into(),
            Some(28) => "The service timed out".into(),
            Some(35 | 60) => "Secure connection failed (check the clock)".into(),
            Some(63) => "Response too large".into(),
            code => format!("Request failed (curl {code:?})"),
        });
    }
    // --max-filesize only applies when the size is announced.
    if out.stdout.len() > max_body + 8 {
        return Err("Response too large".into());
    }
    Ok(split_status(out.stdout))
}

/// curl writes the body, a newline and the HTTP status.
fn split_status(mut body: Vec<u8>) -> Response {
    let split = body.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let status = std::str::from_utf8(&body[split..])
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    body.truncate(split);
    Response { status, body }
}

#[cfg(test)]
mod tests {
    use super::split_status;

    #[test]
    fn the_status_follows_the_body() {
        let r = split_status(b"{\"a\":1}\n\n200".to_vec());
        assert_eq!((r.status, r.body.as_slice()), (200, &b"{\"a\":1}\n"[..]));
        let r = split_status(b"\n404".to_vec());
        assert_eq!((r.status, r.body.len()), (404, 0));
        assert_eq!(split_status(b"garbage".to_vec()).status, 0);
    }
}
