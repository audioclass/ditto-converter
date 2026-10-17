use anyhow::{bail, ensure, Result};
mod acquisition;
pub mod components;
mod provider;
pub use provider::{parse_metadata, Cancellation, OnlineEngine, OnlineEntry};

/// Only canonical provider pages are admitted, never arbitrary extractor inputs.
pub fn validate_url(input: &str) -> Result<&'static str> {
    ensure!(
        input.len() <= 4096 && !input.chars().any(|c| c.is_control() || c == '\\'),
        "invalid URL"
    );
    let u = url::Url::parse(input)?;
    ensure!(
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none(),
        "HTTPS provider URL without credentials required"
    );
    let host = u.host_str().unwrap_or_default();
    let parts: Vec<_> = u.path().split('/').filter(|p| !p.is_empty()).collect();
    match host {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            ensure!(
                matches!(
                    parts.as_slice(),
                    ["watch"] | ["playlist"] | ["shorts", _] | ["live", _] | ["embed", _]
                ),
                "unsupported YouTube page"
            );
            ensure!(
                parts[0] != "watch" || u.query_pairs().any(|(k, v)| k == "v" && !v.is_empty()),
                "missing video ID"
            );
            ensure!(
                parts[0] != "playlist"
                    || u.query_pairs().any(|(k, v)| k == "list" && !v.is_empty()),
                "missing playlist ID"
            );
            Ok("youtube")
        }
        "youtu.be" => {
            ensure!(parts.len() == 1, "missing video ID");
            Ok("youtube")
        }
        "soundcloud.com" | "www.soundcloud.com" => {
            ensure!(
                parts.len() == 2 || (parts.len() == 3 && parts[1] == "sets"),
                "track or playlist URL required"
            );
            ensure!(
                !matches!(
                    parts[0],
                    "you" | "discover" | "search" | "settings" | "connect" | "oauth" | "redirect"
                ),
                "unsupported SoundCloud page"
            );
            Ok("soundcloud")
        }
        _ => bail!("unsupported provider host"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_urls_reject_local_credentials_options_and_spoofed_hosts() {
        for bad in [
            "file:///etc/passwd",
            "--exec=bad",
            "https://youtube.com.evil/watch?v=x",
            "https://u:p@youtube.com/watch?v=x",
            "http://127.0.0.1/",
            "https://youtube.com:444/watch?v=x",
            "https://youtube.com/redirect?q=https://localhost/",
        ] {
            assert!(super::validate_url(bad).is_err(), "{bad}");
        }
        assert_eq!(
            super::validate_url("https://www.youtube.com/watch?v=BaW_jenozKc").unwrap(),
            "youtube"
        );
        assert_eq!(
            super::validate_url("https://soundcloud.com/artist/track").unwrap(),
            "soundcloud"
        );
    }
}
