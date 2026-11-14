use std::path::PathBuf;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use super::ApiError;
use super::auth::http_date;

pub const MEDIA_CACHE: &str = "private, max-age=3600";

pub struct FileReply {
    pub path: PathBuf,
    pub content_type: &'static str,
    pub download: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Range {
    Full,
    Part(u64, u64),
    Unsatisfiable,
    Malformed,
}

pub fn parse_range(header: Option<&str>, size: u64) -> Range {
    let Some(header) = header else {
        return Range::Full;
    };
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return Range::Malformed;
    };
    let ranges: Vec<&str> = spec
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect();
    if ranges.len() != 1 {
        return if ranges.is_empty() {
            Range::Malformed
        } else {
            Range::Full
        };
    }
    let Some((start, end)) = ranges[0].split_once('-') else {
        return Range::Malformed;
    };
    let (start, end) = (start.trim(), end.trim());
    let parsed = match (start.is_empty(), end.is_empty()) {
        (true, true) => return Range::Malformed,
        (true, false) => match end.parse::<u64>() {
            Ok(0) => return Range::Unsatisfiable,
            Ok(suffix) => (size.saturating_sub(suffix), size.saturating_sub(1)),
            Err(_) => return Range::Malformed,
        },
        (false, _) => {
            let Ok(first) = start.parse::<u64>() else {
                return Range::Malformed;
            };
            let last = if end.is_empty() {
                size.saturating_sub(1)
            } else {
                match end.parse::<u64>() {
                    Ok(last) => last.min(size.saturating_sub(1)),
                    Err(_) => return Range::Malformed,
                }
            };
            if !end.is_empty() && end.parse::<u64>().is_ok_and(|e| e < first) {
                return Range::Malformed;
            }
            (first, last)
        }
    };
    if size == 0 || parsed.0 >= size {
        return Range::Unsatisfiable;
    }
    Range::Part(parsed.0, parsed.1)
}

fn etag(mtime: f64, size: u64) -> String {
    let digest = Sha256::digest(format!("{mtime}-{size}").as_bytes());
    format!("\"{}\"", &hex::encode(digest)[..32])
}

fn text(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

pub async fn send(reply: FileReply, request: &HeaderMap) -> Response {
    match serve(reply, request).await {
        Ok(response) => response,
        Err(e) => e.into_response(),
    }
}

async fn serve(reply: FileReply, request: &HeaderMap) -> Result<Response, ApiError> {
    let mut file = match tokio::fs::File::open(&reply.path).await {
        Ok(file) => file,
        Err(_) => return Err(ApiError::new(404, "not found")),
    };
    let meta = file
        .metadata()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let tag = etag(mtime, size);
    let mut headers = HeaderMap::new();
    let mut put = |name: header::HeaderName, value: String| {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    };
    put(header::CACHE_CONTROL, MEDIA_CACHE.into());
    put(header::CONTENT_TYPE, reply.content_type.into());
    put(header::ACCEPT_RANGES, "bytes".into());
    put(header::LAST_MODIFIED, http_date(mtime));
    put(header::ETAG, tag.clone());
    if let Some(name) = &reply.download {
        put(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        );
    }
    let not_modified = request
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == tag)
        });
    if not_modified {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        *response.headers_mut() = headers;
        return Ok(response);
    }
    let range = request.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (status, start, length) = match parse_range(range, size) {
        Range::Full => (StatusCode::OK, 0, size),
        Range::Part(first, last) => {
            headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {first}-{last}/{size}"))
                    .map_err(|e| ApiError::Internal(e.to_string()))?,
            );
            (StatusCode::PARTIAL_CONTENT, first, last - first + 1)
        }
        Range::Unsatisfiable => {
            let mut response = text(StatusCode::RANGE_NOT_SATISFIABLE, "Range Not Satisfiable");
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(response);
        }
        Range::Malformed => return Ok(text(StatusCode::BAD_REQUEST, "Malformed range header.")),
    };
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    let stream = ReaderStream::new(file.take(length));
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_follow_rfc_7233() {
        assert_eq!(parse_range(None, 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=0-99"), 10240), Range::Part(0, 99));
        assert_eq!(parse_range(Some("bytes=10-"), 100), Range::Part(10, 99));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Range::Part(90, 99));
        assert_eq!(parse_range(Some("bytes=0-1000"), 100), Range::Part(0, 99));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=5-1"), 100), Range::Malformed);
        assert_eq!(parse_range(Some("items=0-1"), 100), Range::Malformed);
        assert_eq!(parse_range(Some("bytes=x-1"), 100), Range::Malformed);
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), Range::Full);
        assert_eq!(parse_range(Some("bytes=-"), 100), Range::Malformed);
    }
}
