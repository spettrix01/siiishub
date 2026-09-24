//! Remote files read over several connections at once. One connection to a
//! debrid CDN can bring less than a 4K film plays, when the server is far
//! away or the connection slows down for a while; a few of them, each on its
//! own part of the file, bring several times that. Every byte range asked of
//! a remote file (by ffmpeg, through a relay of its own on 127.0.0.1, or by
//! the browser playing the file as it is) comes in chunks fetched a few at a
//! time ahead of the reader and passed on in order.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;

/// Chunks fetched at once, ahead of the reader.
const PARALLEL: usize = 4;
/// A chunk once the reading is under way.
const CHUNK: u64 = 4 << 20;
/// The first chunk of a request, doubling up to `CHUNK`: ffmpeg reads little
/// before most seeks (headers, the index at the end of the file).
const FIRST_CHUNK: u64 = 256 << 10;
/// Tries per chunk.
const TRIES: u32 = 3;
/// Longest wait for one chunk.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

/// The relay ffmpeg reads remote files through, on 127.0.0.1.
pub struct Relay {
    client: reqwest::Client,
    port: u16,
    files: Mutex<HashMap<String, Weak<Upstream>>>,
}

impl Relay {
    pub async fn start(client: reqwest::Client) -> anyhow::Result<Arc<Self>> {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let relay = Arc::new(Self {
            client,
            port: listener.local_addr()?.port(),
            files: Mutex::default(),
        });
        let router = Router::new()
            .route("/{token}", get(relay_file))
            .with_state(relay.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!("[relay] stopped: {e}");
            }
        });
        Ok(relay)
    }

    /// The remote file at `url`, known to the relay while something holds it.
    pub fn upstream(&self, url: &str) -> Arc<Upstream> {
        let token = super::random_hex(16);
        let upstream = Arc::new(Upstream {
            client: self.client.clone(),
            url: url.to_string(),
            local: format!("http://127.0.0.1:{}/{token}", self.port),
            target: tokio::sync::Mutex::new(Target::Unknown),
        });
        let mut files = self.files.lock();
        files.retain(|_, file| file.strong_count() > 0);
        files.insert(token, Arc::downgrade(&upstream));
        upstream
    }
}

async fn relay_file(State(relay): State<Arc<Relay>>, Path(token): Path<String>, headers: HeaderMap) -> Response {
    let upstream = relay.files.lock().get(&token).and_then(Weak::upgrade);
    match upstream {
        Some(upstream) => upstream.serve(headers.get(header::RANGE)).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Where a remote file is, once known.
#[derive(Clone)]
enum Target {
    Unknown,
    /// The address after the redirects (an addon's resolver sends to the
    /// debrid CDN), the size and the type.
    Ranges { url: String, size: u64, kind: Option<HeaderValue> },
    /// No byte ranges there: one connection, as it is.
    Whole,
}

pub struct Upstream {
    client: reqwest::Client,
    url: String,
    local: String,
    target: tokio::sync::Mutex<Target>,
}

impl Upstream {
    /// Its address on the relay, for ffmpeg and ffprobe.
    pub fn local_url(&self) -> &str {
        &self.local
    }

    /// Answers a request for the file, or for the part `range` asks.
    pub async fn serve(self: &Arc<Self>, range: Option<&HeaderValue>) -> Response {
        let (size, kind) = match self.target().await {
            Ok(Target::Ranges { size, kind, .. }) => (size, kind),
            Ok(_) => return proxy(&self.client, &self.url, range).await,
            Err(e) => {
                // Without the URL: debrid links open the file to whoever has them.
                tracing::warn!("[relay] upstream request failed: {}", e.without_url());
                return StatusCode::BAD_GATEWAY.into_response();
            }
        };
        let (first, last) = match range.and_then(|r| r.to_str().ok()) {
            None => (0, size.saturating_sub(1)),
            Some(range) => match parse_range(range, size) {
                Some(span) => span,
                None => {
                    return Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                        .body(Body::empty())
                        .unwrap_or_else(|_| StatusCode::RANGE_NOT_SATISFIABLE.into_response());
                }
            },
        };
        let kind = kind.unwrap_or(HeaderValue::from_static("application/octet-stream"));
        let mut response = Response::builder()
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_TYPE, kind)
            .header(header::CONTENT_LENGTH, last + 1 - first)
            .header(header::CACHE_CONTROL, "no-store");
        response = if range.is_some() {
            response
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {first}-{last}/{size}"))
        } else {
            response.status(StatusCode::OK)
        };
        response
            .body(Body::from_stream(self.clone().read(first, last)))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }

    /// Where the file is: asked once, with one byte, following the redirects.
    async fn target(&self) -> Result<Target, reqwest::Error> {
        let mut target = self.target.lock().await;
        if !matches!(*target, Target::Unknown) {
            return Ok(target.clone());
        }
        let reply = self
            .client
            .get(&self.url)
            .header(header::RANGE, "bytes=0-0")
            .timeout(CHUNK_TIMEOUT)
            .send()
            .await?;
        let size = (reply.status() == StatusCode::PARTIAL_CONTENT)
            .then(|| reply.headers().get(header::CONTENT_RANGE))
            .flatten()
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit_once('/'))
            .and_then(|(_, total)| total.trim().parse::<u64>().ok())
            .filter(|&size| size > 0);
        *target = match size {
            Some(size) => Target::Ranges {
                url: reply.url().to_string(),
                size,
                kind: reply.headers().get(header::CONTENT_TYPE).cloned(),
            },
            None => Target::Whole,
        };
        Ok(target.clone())
    }

    /// The file from byte `first` to byte `last`, in chunks fetched
    /// `PARALLEL` at a time and yielded in order.
    fn read(self: Arc<Self>, first: u64, last: u64) -> impl Stream<Item = std::io::Result<Bytes>> {
        let spans = std::iter::successors(Some((first, FIRST_CHUNK)), move |&(at, size)| {
            let next = at + size;
            (next <= last).then(|| (next, (size * 2).min(CHUNK)))
        })
        .map(move |(at, size)| (at, (at + size - 1).min(last)));
        futures::stream::iter(spans)
            .map(move |(from, to)| {
                let upstream = self.clone();
                async move { upstream.chunk(from, to).await }
            })
            .buffered(PARALLEL)
    }

    async fn chunk(&self, from: u64, to: u64) -> std::io::Result<Bytes> {
        let mut problem = String::new();
        for attempt in 0..TRIES {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
            }
            let url = match self.target().await {
                Ok(Target::Ranges { url, .. }) => url,
                Ok(_) => return Err(std::io::Error::other("the file no longer takes byte ranges")),
                Err(e) => {
                    problem = e.without_url().to_string();
                    continue;
                }
            };
            let reply = self
                .client
                .get(&url)
                .header(header::RANGE, format!("bytes={from}-{to}"))
                .timeout(CHUNK_TIMEOUT)
                .send()
                .await;
            match reply {
                Ok(reply) if reply.status() == StatusCode::PARTIAL_CONTENT => match reply.bytes().await {
                    Ok(bytes) if bytes.len() as u64 == to + 1 - from => return Ok(bytes),
                    Ok(bytes) => problem = format!("{} bytes instead of {}", bytes.len(), to + 1 - from),
                    Err(e) => problem = e.without_url().to_string(),
                },
                Ok(reply) => {
                    problem = format!("HTTP {}", reply.status());
                    // A debrid link that expired: the original address
                    // gives a new one.
                    if matches!(reply.status().as_u16(), 401 | 403 | 404 | 410) {
                        *self.target.lock().await = Target::Unknown;
                    }
                }
                Err(e) => problem = e.without_url().to_string(),
            }
        }
        tracing::warn!("[relay] bytes {from}-{to}: {problem}");
        Err(std::io::Error::other(problem))
    }
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` within a file of `size` bytes: the
/// first and last byte. Several ranges are not taken.
fn parse_range(range: &str, size: u64) -> Option<(u64, u64)> {
    let spec = range.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (first, last) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (size.saturating_sub(n), size - 1)
    } else {
        let first: u64 = a.parse().ok()?;
        let last = if b.is_empty() { size - 1 } else { b.parse::<u64>().ok()?.min(size - 1) };
        (first, last)
    };
    (first <= last && first < size).then_some((first, last))
}

/// The file as the server at `url` sends it, over one connection: for what
/// takes no byte ranges, and for the torrent session on 127.0.0.1.
pub async fn proxy(client: &reqwest::Client, url: &str, range: Option<&HeaderValue>) -> Response {
    let mut upstream = client.get(url);
    if let Some(range) = range {
        upstream = upstream.header(header::RANGE, range.clone());
    }
    let reply = match upstream.send().await {
        Ok(reply) => reply,
        Err(e) => {
            tracing::warn!("[relay] upstream request failed: {}", e.without_url());
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let mut response = Response::builder().status(reply.status());
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::LAST_MODIFIED,
        header::ETAG,
    ] {
        if let Some(value) = reply.headers().get(&name) {
            response = response.header(name, value.clone());
        }
    }
    response
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(reply.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }
}
