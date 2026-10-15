//! HTTP with reqwest (rustls, no OpenSSL): a client that only talks to allow-listed https hosts, bounded bodies, and a downloader that
//! checks size and SHA-256 while streaming and never leaves a partial file behind.

use crate::config::ReleaseConfig;
use crate::error::{Kind, Result, UpdateError};
use crate::{hex, Cancel};
use futures_util::StreamExt;
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

pub const API_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    pub cfg: Arc<ReleaseConfig>,
}

/// An answer, whatever its status. The body is bounded.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
    pub rate_remaining: Option<u64>,
    /// Unix seconds when the rate limit resets (`x-ratelimit-reset`), or now + `retry-after`.
    pub rate_reset: Option<i64>,
}

pub fn map_reqwest(e: reqwest::Error) -> UpdateError {
    if e.is_redirect() {
        return UpdateError::new(Kind::Policy, format!("redirect refused: {e}"));
    }
    if e.is_builder() {
        return UpdateError::new(Kind::Policy, format!("bad request: {e}"));
    }
    if e.is_connect() || e.is_timeout() || e.is_request() {
        return UpdateError::new(Kind::Offline, format!("{e}"));
    }
    if e.is_body() || e.is_decode() {
        return UpdateError::new(Kind::Offline, format!("connection lost: {e}"));
    }
    UpdateError::new(Kind::Http, format!("{e}"))
}

impl Http {
    pub fn new(cfg: Arc<ReleaseConfig>) -> Result<Http> {
        let policy_cfg = cfg.clone();
        let policy = Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if policy_cfg.check_url(attempt.url().as_str()).is_ok() {
                attempt.follow()
            } else {
                attempt.error("redirect to a host that is not allowed")
            }
        });
        let client = reqwest::Client::builder()
            .user_agent(cfg.user_agent.clone())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .redirect(policy)
            .build()
            .map_err(|e| UpdateError::new(Kind::Io, format!("cannot build the HTTP client: {e}")))?;
        Ok(Http { client, cfg })
    }

    /// GET with an optional `If-None-Match`. Any HTTP status comes back as a [`Reply`]; only transport problems are errors.
    pub async fn get(&self, url: &str, if_none_match: Option<&str>, accept: &str, max_body: usize, timeout: Duration) -> Result<Reply> {
        let url = self.cfg.check_url(url)?;
        let mut req = self.client.get(url).header("Accept", accept).header("X-GitHub-Api-Version", "2022-11-28").timeout(timeout);
        if let Some(tag) = if_none_match {
            req = req.header("If-None-Match", tag);
        }
        let resp = req.send().await.map_err(map_reqwest)?;
        let status = resp.status().as_u16();
        let h = resp.headers();
        let etag = h.get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
        let rate_remaining = h.get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok());
        let mut rate_reset = h.get("x-ratelimit-reset").and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<i64>().ok());
        if let Some(secs) = h.get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<i64>().ok()) {
            rate_reset = Some(crate::now_unix() + secs);
        }
        if let Some(len) = resp.content_length() {
            if len as usize > max_body {
                return Err(UpdateError::new(Kind::TooLarge, format!("the answer is {len} bytes (limit {max_body})")));
            }
        }
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_reqwest)?;
            if body.len() + chunk.len() > max_body {
                return Err(UpdateError::new(Kind::TooLarge, format!("the answer is longer than {max_body} bytes")));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply { status, etag, body, rate_remaining, rate_reset })
    }

    /// Streams `url` into `dest` (a temporary name the caller owns), checking while it goes that the size is exactly `size` and the
    /// SHA-256 is `sha256`. On any failure `dest` is deleted. Returns the number of bytes written.
    pub async fn download(&self, url: &str, dest: &Path, size: u64, sha256: &str, cancel: &Cancel, progress: &(dyn Fn(u64, u64) + Send + Sync)) -> Result<u64> {
        let r = self.download_inner(url, dest, size, sha256, cancel, progress).await;
        if r.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
        }
        r
    }

    async fn download_inner(&self, url: &str, dest: &Path, size: u64, sha256: &str, cancel: &Cancel, progress: &(dyn Fn(u64, u64) + Send + Sync)) -> Result<u64> {
        let url = self.cfg.check_asset_url(url)?;
        let resp = self.client.get(url).header("Accept", "application/octet-stream").send().await.map_err(map_reqwest)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(UpdateError::new(Kind::Http, format!("download answered HTTP {}", status.as_u16())));
        }
        if let Some(len) = resp.content_length() {
            if len != size {
                return Err(UpdateError::new(Kind::Size, format!("the server announces {len} bytes, the manifest says {size}")));
            }
        }
        if let Some(p) = dest.parent() {
            tokio::fs::create_dir_all(p).await.map_err(|e| UpdateError::io(p.display(), &e))?;
        }
        let mut file = tokio::fs::File::create(dest).await.map_err(|e| UpdateError::io(dest.display(), &e))?;
        let mut hasher = Sha256::new();
        let mut done: u64 = 0;
        let mut last = Instant::now();
        progress(0, size);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if cancel.is_cancelled() {
                return Err(UpdateError::new(Kind::Cancelled, "download cancelled"));
            }
            let chunk = chunk.map_err(map_reqwest)?;
            done += chunk.len() as u64;
            if done > size {
                return Err(UpdateError::new(Kind::Size, format!("more than the {size} bytes the manifest announces")));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| UpdateError::io(dest.display(), &e))?;
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                progress(done, size);
            }
        }
        file.flush().await.map_err(|e| UpdateError::io(dest.display(), &e))?;
        drop(file);
        if done != size {
            return Err(UpdateError::new(Kind::Size, format!("the download ended after {done} of {size} bytes")));
        }
        let got = hex(&hasher.finalize());
        if !got.eq_ignore_ascii_case(sha256) {
            return Err(UpdateError::new(Kind::Hash, format!("sha256 is {got}, the manifest says {sha256}")));
        }
        progress(done, size);
        Ok(done)
    }
}

/// SHA-256 of a file, streamed (call from a blocking thread for big files). Returns (lower-case hex, size).
pub fn sha256_file(path: &Path, cancel: Option<&Cancel>) -> Result<(String, u64)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| UpdateError::io(path.display(), &e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if let Some(c) = cancel {
            if c.is_cancelled() {
                return Err(UpdateError::new(Kind::Cancelled, "cancelled"));
            }
        }
        let n = f.read(&mut buf).map_err(|e| UpdateError::io(path.display(), &e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex(&h.finalize()), total))
}
