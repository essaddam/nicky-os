//! Ephemeral, authenticated media transport for the isolated POC.
//!
//! Limits count retained payload bytes, including responses already handed to
//! HTTP. Revocation/expiry prevents new requests; bytes already sent or handed
//! to an active response cannot be recalled. HTTP/TCP buffer overhead is not
//! included in the payload budget. This module never fetches remote content.

use std::{
    collections::HashMap,
    fs::File,
    future::IntoFuture,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use anyhow::{Context as _, anyhow, bail};
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Method, Response, StatusCode, header},
    routing::get,
};
use bytes::Bytes;
use http_body::{Frame, SizeHint};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

const MAX_ITEMS: usize = 4;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const FILE_CHUNK: usize = 64 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TTL: Duration = Duration::from_secs(60);
const MIME_TYPES: &[&str] = &[
    "audio/wav",
    "audio/flac",
    "video/mp4",
    "application/octet-stream",
];

#[derive(Clone)]
pub struct MediaStore {
    inner: Arc<Inner>,
}

struct Inner {
    items: Mutex<HashMap<String, Item>>,
    retained_bytes: Arc<AtomicUsize>,
    responses: Arc<Semaphore>,
}

struct Item {
    token: String,
    mime: &'static str,
    expires: Instant,
    data: MediaData,
    length: usize,
    active: Arc<AtomicBool>,
}

#[derive(Clone)]
enum MediaData {
    Memory(Bytes),
    File(Arc<File>),
}

impl Drop for Item {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

// Deliberately no Debug/Serialize: credentials must not enter diagnostics.
pub struct MediaTicket {
    pub id: String,
    pub token: String,
    pub length: usize,
    pub mime: String,
}

struct Payload {
    data: Box<[u8]>,
    retained_bytes: Arc<AtomicUsize>,
}

impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.retained_bytes
            .fetch_sub(self.data.len(), Ordering::AcqRel);
    }
}

impl Default for MediaStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                items: Mutex::new(HashMap::new()),
                retained_bytes: Arc::new(AtomicUsize::new(0)),
                responses: Arc::new(Semaphore::new(MAX_ITEMS)),
            }),
        }
    }

    pub fn insert(&self, mime: &str, data: Vec<u8>, ttl: Duration) -> anyhow::Result<MediaTicket> {
        if data.is_empty() || data.len() > MAX_BYTES {
            bail!("media payload must contain 1 byte to 8 MiB");
        }
        self.purge();
        let length = data.len();
        // Reservation is atomic because file readers allocate concurrently.
        reserve_bytes(&self.inner.retained_bytes, length)?;
        let payload = Payload {
            data: data.into_boxed_slice(),
            retained_bytes: self.inner.retained_bytes.clone(),
        };
        self.insert_data(
            mime,
            MediaData::Memory(Bytes::from_owner(payload)),
            length,
            ttl,
        )
    }

    /// Accept an already opened regular file. No client path or URL is accepted.
    /// The caller must not modify its contents during the ticket's lifetime.
    pub fn insert_file(
        &self,
        mime: &str,
        file: File,
        ttl: Duration,
    ) -> anyhow::Result<MediaTicket> {
        let metadata = file.metadata().context("cannot inspect media file")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE_BYTES {
            bail!("media file must be regular and contain 1 byte to 4 GiB");
        }
        let length = usize::try_from(metadata.len()).context("media file length unsupported")?;
        self.insert_data(mime, MediaData::File(Arc::new(file)), length, ttl)
    }

    fn insert_data(
        &self,
        mime: &str,
        data: MediaData,
        length: usize,
        ttl: Duration,
    ) -> anyhow::Result<MediaTicket> {
        let mime = *MIME_TYPES
            .iter()
            .find(|allowed| **allowed == mime)
            .ok_or_else(|| anyhow!("unsupported media MIME type"))?;
        if ttl.is_zero() || ttl > MAX_TTL {
            bail!("media lifetime must be greater than zero and at most 60 seconds");
        }
        let mut items = self
            .inner
            .items
            .lock()
            .map_err(|_| anyhow!("media store unavailable"))?;
        items.retain(|_, item| item.expires > Instant::now());
        if items.len() >= MAX_ITEMS {
            bail!("media ticket quota reached");
        }
        let id = loop {
            let candidate = random_hex::<16>()?;
            if !items.contains_key(&candidate) {
                break candidate;
            }
        };
        let token = random_hex::<32>()?;
        let expires = Instant::now() + ttl;
        items.insert(
            id.clone(),
            Item {
                token: token.clone(),
                mime,
                expires,
                data,
                length,
                active: Arc::new(AtomicBool::new(true)),
            },
        );
        Ok(MediaTicket {
            id,
            token,
            length,
            mime: mime.to_owned(),
        })
    }

    pub fn revoke(&self, id: &str) {
        // Recovering this lock only deletes data, never grants new access.
        self.inner
            .items
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(id);
    }

    fn purge(&self) {
        self.inner
            .items
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .retain(|_, item| item.expires > Instant::now());
    }

    fn revoke_all(&self) {
        self.inner
            .items
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clear();
    }
}

fn reserve_payload(counter: &Arc<AtomicUsize>, length: usize) -> anyhow::Result<Payload> {
    reserve_bytes(counter, length)?;
    Ok(Payload {
        data: vec![0; length].into_boxed_slice(),
        retained_bytes: counter.clone(),
    })
}

fn reserve_bytes(counter: &Arc<AtomicUsize>, length: usize) -> anyhow::Result<()> {
    counter
        .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(length).filter(|total| *total <= MAX_BYTES)
        })
        .map_err(|_| anyhow!("retained media payload quota reached"))?;
    Ok(())
}

fn random_hex<const N: usize>() -> anyhow::Result<String> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| anyhow!("secure random source unavailable"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(N * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(output)
}

fn router(store: MediaStore) -> Router {
    Router::new()
        .route("/media/{id}", get(media))
        .with_state(store)
}

/// Serve until cancellation; purge runs only while this future is active.
/// Waits at most two seconds for graceful shutdown. Axum may still own an
/// already accepted connection after this future returns; the caller owns
/// the enclosing runtime's lifetime. Already delivered bytes remain delivered.
pub async fn serve(
    store: MediaStore,
    listener: tokio::net::TcpListener,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    struct RevokeOnStop(MediaStore);
    impl Drop for RevokeOnStop {
        fn drop(&mut self) {
            self.0.revoke_all();
        }
    }
    let _revoke_on_stop = RevokeOnStop(store.clone());
    let shutdown = cancel.clone();
    let server = axum::serve(listener, router(store.clone()))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .into_future();
    tokio::pin!(server);
    let mut purge = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            result = &mut server => return result.context("media HTTP server failed"),
            _ = purge.tick() => store.purge(),
            _ = cancel.cancelled() => {
                store.revoke_all();
                return match tokio::time::timeout(Duration::from_secs(2), &mut server).await {
                    Ok(result) => result.context("media HTTP server failed during shutdown"),
                    Err(_) => Ok(()),
                };
            }
        }
    }
}

async fn media(
    State(store): State<MediaStore>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response<Body> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return empty(StatusCode::NOT_FOUND);
    }
    let mut authorization = headers.get_all(header::AUTHORIZATION).iter();
    let token = authorization
        .next()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let Some(token) = token.filter(|value| value.len() == 64) else {
        return unauthorized();
    };
    if authorization.next().is_some() {
        return unauthorized();
    }
    let (data, mime, length, active, expires) = {
        let Ok(mut items) = store.inner.items.lock() else {
            return empty(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let Some(item) = items.get(&id) else {
            return empty(StatusCode::NOT_FOUND);
        };
        if item.expires <= Instant::now() {
            items.remove(&id);
            return empty(StatusCode::NOT_FOUND);
        }
        if !bool::from(item.token.as_bytes().ct_eq(token.as_bytes())) {
            return unauthorized();
        }
        (
            item.data.clone(),
            item.mime,
            item.length,
            item.active.clone(),
            item.expires,
        )
    };
    // RFC 9110 defines Range for GET. HEAD reports the complete representation.
    let range = if method == Method::HEAD {
        Ok(None)
    } else {
        let mut values = headers.get_all(header::RANGE).iter();
        let result = match values.next() {
            None => Ok(None),
            Some(value) => value
                .to_str()
                .map_err(|_| ())
                .and_then(|value| parse_range(value, length))
                .map(Some),
        };
        if values.next().is_some() {
            Err(())
        } else {
            result
        }
    };
    let range = match range {
        Ok(range) => range,
        Err(()) => {
            let mut response = empty(StatusCode::RANGE_NOT_SATISFIABLE);
            set_header(
                &mut response,
                header::CONTENT_RANGE,
                &format!("bytes */{length}"),
            );
            response
                .headers_mut()
                .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            return response;
        }
    };
    let (start, end) = range.unwrap_or((0, length - 1));
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        let Ok(permit) = store.inner.responses.clone().try_acquire_owned() else {
            return empty(StatusCode::SERVICE_UNAVAILABLE);
        };
        let body = match data {
            MediaData::Memory(data) => Body::from(data.slice(start..=end)),
            MediaData::File(file) => file_body(
                file,
                start,
                end,
                active.clone(),
                expires,
                store.inner.retained_bytes.clone(),
            ),
        };
        Body::new(MediaBody {
            body,
            active,
            expires,
            _permit: permit,
        })
    };
    let mut response = Response::new(body);
    *response.status_mut() = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    set_header(
        &mut response,
        header::CONTENT_LENGTH,
        &(end - start + 1).to_string(),
    );
    if range.is_some() {
        set_header(
            &mut response,
            header::CONTENT_RANGE,
            &format!("bytes {start}-{end}/{length}"),
        );
    }
    response
}

fn empty(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    response
}

fn unauthorized() -> Response<Body> {
    let mut response = empty(StatusCode::UNAUTHORIZED);
    response
        .headers_mut()
        .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    response
}

fn set_header(response: &mut Response<Body>, name: header::HeaderName, value: &str) {
    // Only numeric values and fixed ASCII syntax generated above reach here.
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// A single range only; inclusive bounds and decimal numbers without signs.
fn parse_range(value: &str, length: usize) -> Result<(usize, usize), ()> {
    let range = value.strip_prefix("bytes=").ok_or(())?;
    let (start, end) = range.split_once('-').ok_or(())?;
    let decimal = |text: &str| {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
        text.parse::<usize>().map_err(|_| ())
    };
    if length == 0 {
        return Err(());
    }
    if start.is_empty() {
        let suffix = decimal(end)?;
        if suffix == 0 {
            return Err(());
        }
        return Ok((length.saturating_sub(suffix), length - 1));
    }
    let start = decimal(start)?;
    if start >= length {
        return Err(());
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        decimal(end)?.min(length - 1)
    };
    if start > end {
        return Err(());
    }
    Ok((start, end))
}

// Keeping the permit in the response body bounds concurrent slow consumers.
struct MediaBody {
    body: Body,
    active: Arc<AtomicBool>,
    expires: Instant,
    _permit: OwnedSemaphorePermit,
}

impl http_body::Body for MediaBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if !self.active.load(Ordering::Acquire) || Instant::now() >= self.expires {
            return Poll::Ready(None);
        }
        Pin::new(&mut self.body).poll_frame(_cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

fn file_body(
    file: Arc<File>,
    start: usize,
    end: usize,
    active: Arc<AtomicBool>,
    expires: Instant,
    counter: Arc<AtomicUsize>,
) -> Body {
    let state = (file, start, end, active, expires, counter);
    Body::from_stream(futures_util::stream::try_unfold(
        state,
        |(file, offset, end, active, expires, counter)| async move {
            if offset > end || !active.load(Ordering::Acquire) || Instant::now() >= expires {
                return Ok::<_, std::io::Error>(None);
            }
            let count = FILE_CHUNK.min(end - offset + 1);
            let payload = reserve_payload(&counter, count).map_err(std::io::Error::other)?;
            let source = file.clone();
            let allowed = active.clone();
            let read = tokio::task::spawn_blocking(move || {
                use std::os::unix::fs::FileExt;
                let mut payload = payload;
                let mut filled = 0;
                while filled < count {
                    if !allowed.load(Ordering::Acquire) || Instant::now() >= expires {
                        return Err(std::io::Error::other("media access ended"));
                    }
                    match source.read_at(&mut payload.data[filled..], (offset + filled) as u64) {
                        Ok(0) => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "media file changed",
                            ));
                        }
                        Ok(read) => filled += read,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    }
                }
                Ok(Bytes::from_owner(payload))
            });
            // Bounding the await cannot interrupt a filesystem syscall already in progress.
            let remaining = expires.saturating_duration_since(Instant::now());
            let bytes = tokio::time::timeout(remaining, read)
                .await
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::TimedOut, "media access expired")
                })?
                .map_err(std::io::Error::other)??;
            if !active.load(Ordering::Acquire) || Instant::now() >= expires {
                return Ok(None);
            }
            Ok(Some((
                bytes,
                (file, offset + count, end, active, expires, counter),
            )))
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    fn ticket(store: &MediaStore, length: usize) -> MediaTicket {
        store
            .insert("audio/wav", vec![42; length], MAX_TTL)
            .unwrap()
    }

    #[test]
    fn range_forms_and_rejections() {
        for (input, expected) in [
            ("bytes=2-4", (2, 4)),
            ("bytes=7-", (7, 9)),
            ("bytes=-3", (7, 9)),
            ("bytes=0-99", (0, 9)),
            ("bytes=-99", (0, 9)),
        ] {
            assert_eq!(parse_range(input, 10), Ok(expected));
        }
        for input in [
            "bytes=10-",
            "bytes=-0",
            "bytes=4-2",
            "bytes=1-2,4-5",
            "items=0-1",
            "bytes=+1-2",
            "bytes= 1-2",
            "bytes=-",
            "bytes=0-184467440737095516160",
        ] {
            assert_eq!(parse_range(input, 10), Err(()), "{input}");
        }
        assert_eq!(parse_range("bytes=0-", 0), Err(()));
    }

    #[test]
    fn quotas_count_revoked_payloads_still_held_by_responses() {
        let store = MediaStore::new();
        let first = ticket(&store, MAX_BYTES);
        let held = store.inner.items.lock().unwrap()[&first.id].data.clone();
        store.revoke(&first.id);
        assert!(store.insert("audio/wav", vec![0], MAX_TTL).is_err());
        drop(held);
        assert!(store.insert("audio/wav", vec![0], MAX_TTL).is_ok());
        let store = MediaStore::new();
        let tickets: Vec<_> = (0..MAX_ITEMS).map(|_| ticket(&store, 1)).collect();
        assert!(store.insert("audio/wav", vec![0], MAX_TTL).is_err());
        store.revoke(&tickets[0].id);
        assert!(store.insert("video/mp4", vec![0], MAX_TTL).is_ok());
    }

    #[test]
    fn validates_lifetime_payload_and_mime() {
        let store = MediaStore::new();
        for ttl in [Duration::ZERO, MAX_TTL + Duration::from_nanos(1)] {
            assert!(store.insert("audio/wav", vec![0], ttl).is_err());
        }
        assert!(store.insert("text/html", vec![0], MAX_TTL).is_err());
        assert!(store.insert("audio/wav", vec![], MAX_TTL).is_err());
        assert!(
            store
                .insert("audio/wav", vec![0; MAX_BYTES + 1], MAX_TTL)
                .is_err()
        );
    }

    async fn request(
        store: &MediaStore,
        ticket: &MediaTicket,
        method: Method,
        authorization: Option<&str>,
        range: Option<&str>,
    ) -> Response<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/media/{}", ticket.id));
        if let Some(value) = authorization {
            builder = builder.header(header::AUTHORIZATION, value);
        }
        if let Some(value) = range {
            builder = builder.header(header::RANGE, value);
        }
        router(store.clone())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn authentication_ranges_head_and_revocation() {
        let store = MediaStore::new();
        let ticket = ticket(&store, 10);
        let auth = format!("Bearer {}", ticket.token);
        assert_eq!(
            request(&store, &ticket, Method::GET, None, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            request(
                &store,
                &ticket,
                Method::GET,
                Some(&format!("Bearer {}", "0".repeat(64))),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let full = request(&store, &ticket, Method::GET, Some(&auth), None).await;
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(to_bytes(full.into_body(), 10).await.unwrap().len(), 10);
        let partial = request(&store, &ticket, Method::GET, Some(&auth), Some("bytes=2-4")).await;
        assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(partial.headers()[header::CONTENT_RANGE], "bytes 2-4/10");
        assert_eq!(partial.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(to_bytes(partial.into_body(), 10).await.unwrap().len(), 3);
        let head = request(
            &store,
            &ticket,
            Method::HEAD,
            Some(&auth),
            Some("bytes=2-4"),
        )
        .await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_LENGTH], "10");
        assert!(to_bytes(head.into_body(), 10).await.unwrap().is_empty());
        let invalid = request(&store, &ticket, Method::GET, Some(&auth), Some("bytes=10-")).await;
        assert_eq!(invalid.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(invalid.headers()[header::CONTENT_RANGE], "bytes */10");
        store.revoke(&ticket.id);
        assert_eq!(
            request(&store, &ticket, Method::GET, Some(&auth), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn active_response_quota_is_released_on_drop() {
        let store = MediaStore::new();
        let ticket = ticket(&store, 10);
        let auth = format!("Bearer {}", ticket.token);
        let mut active = Vec::new();
        for _ in 0..MAX_ITEMS {
            let response = request(&store, &ticket, Method::GET, Some(&auth), None).await;
            assert_eq!(response.status(), StatusCode::OK);
            active.push(response);
        }
        assert_eq!(
            request(&store, &ticket, Method::GET, Some(&auth), None)
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        active.pop();
        assert_eq!(
            request(&store, &ticket, Method::GET, Some(&auth), None)
                .await
                .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn expired_tickets_are_inaccessible_and_reclaimed() {
        let store = MediaStore::new();
        let ticket = store
            .insert("audio/flac", vec![1], Duration::from_millis(1))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        let auth = format!("Bearer {}", ticket.token);
        assert_eq!(
            request(&store, &ticket, Method::GET, Some(&auth), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        store.purge();
        assert_eq!(store.inner.retained_bytes.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn opened_file_ranges_span_chunks_without_cursor_interference() {
        use std::io::Write;
        let mut file = tempfile::tempfile().unwrap();
        let data: Vec<u8> = (0..FILE_CHUNK * 3 + 13)
            .map(|index| (index % 251) as u8)
            .collect();
        file.write_all(&data).unwrap();
        let store = MediaStore::new();
        let ticket = store.insert_file("video/mp4", file, MAX_TTL).unwrap();
        assert_eq!(store.inner.retained_bytes.load(Ordering::Acquire), 0);
        assert_eq!(
            request(&store, &ticket, Method::GET, None, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let auth = format!("Bearer {}", ticket.token);
        let end = FILE_CHUNK * 2 + 7;
        let response = request(
            &store,
            &ticket,
            Method::GET,
            Some(&auth),
            Some(&format!("bytes=9-{end}")),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let suffix = request(&store, &ticket, Method::GET, Some(&auth), Some("bytes=-5")).await;
        assert_eq!(
            &to_bytes(suffix.into_body(), 5).await.unwrap()[..],
            &data[data.len() - 5..]
        );
        assert_eq!(
            &to_bytes(response.into_body(), data.len()).await.unwrap()[..],
            &data[9..=end]
        );
        assert_eq!(store.inner.retained_bytes.load(Ordering::Acquire), 0);
        let active = request(&store, &ticket, Method::GET, Some(&auth), None).await;
        store.revoke(&ticket.id);
        assert!(
            to_bytes(active.into_body(), data.len())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            request(&store, &ticket, Method::GET, Some(&auth), None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn file_tickets_share_quota_and_reject_empty_or_oversized_files() {
        let store = MediaStore::new();
        let file = tempfile::tempfile().unwrap();
        assert!(store.insert_file("video/mp4", file, MAX_TTL).is_err());
        let file = tempfile::tempfile().unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(store.insert_file("video/mp4", file, MAX_TTL).is_err());
        let tickets: Vec<_> = (0..MAX_ITEMS).map(|_| ticket(&store, 1)).collect();
        let file = tempfile::tempfile().unwrap();
        file.set_len(1).unwrap();
        assert!(store.insert_file("video/mp4", file, MAX_TTL).is_err());
        store.revoke(&tickets[0].id);
        let file = tempfile::tempfile().unwrap();
        file.set_len(1).unwrap();
        assert!(store.insert_file("video/mp4", file, MAX_TTL).is_ok());
    }
}
