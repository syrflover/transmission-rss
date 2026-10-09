//! Asking Chromium's DevTools port for its browser socket.
//!
//! One plain HTTP request on loopback, so it is written out here instead of
//! pulling an HTTP client into the container's launcher.

use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// The path of the browser-level DevTools WebSocket (`/devtools/browser/<id>`)
/// when `127.0.0.1:port` answers `/json/version` with one.
pub(super) async fn browser_socket_path(port: u16) -> Option<String> {
    let body = tokio::time::timeout(Duration::from_secs(2), get_version(port))
        .await
        .ok()?
        .ok()?;
    socket_path_in(&body)
}

fn socket_path_in(body: &str) -> Option<String> {
    // The body is JSON; whatever frames it (a chunked transfer's sizes) has no
    // braces, so the object is what lies between the outer ones.
    let json = body.get(body.find('{')?..=body.rfind('}')?)?;
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let url = url::Url::parse(value.get("webSocketDebuggerUrl")?.as_str()?).ok()?;
    // Only the path: the host is always the loopback this was asked on.
    Some(url.path().to_owned()).filter(|p| p.starts_with("/devtools/"))
}

async fn get_version(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    // DevTools refuses a Host that is not an IP address or localhost.
    let request = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    // Chromium keeps the connection open after its answer whatever the request
    // says, so the answer ends where its `Content-Length` says, not at the end
    // of the stream.
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await?;
        raw.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&raw);
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            if !(head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200")) {
                return Err(std::io::Error::other("DevTools did not answer 200"));
            }
            let wanted = head.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            let have = raw.len() - (head.len() + 4);
            if wanted.is_some_and(|wanted| have >= wanted) || read == 0 {
                return Ok(body.to_owned());
            }
        } else if read == 0 {
            return Err(std::io::Error::other("DevTools closed without an answer"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_socket_path() {
        let body = r#"{"Browser":"Chrome/152","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/abc-123"}"#;
        assert_eq!(
            socket_path_in(body).as_deref(),
            Some("/devtools/browser/abc-123")
        );
        // Chunked framing around the object.
        let chunked = format!("5a\r\n{body}\r\n0\r\n\r\n");
        assert_eq!(
            socket_path_in(&chunked).as_deref(),
            Some("/devtools/browser/abc-123")
        );
    }

    /// Chromium answers `/json/version` and keeps the connection open.
    #[tokio::test]
    async fn it_does_not_wait_for_the_connection_to_close() {
        let served = trss_core::loopback::serve(|listener| async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            let body = r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:1/devtools/browser/kept-open"}"#;
            let answer = format!(
                "HTTP/1.1 200 OK\r\nContent-Type:application/json\r\nContent-Length:{}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(answer.as_bytes()).await.unwrap();
            // Open: no close, no more bytes.
            tokio::time::sleep(Duration::from_secs(30)).await;
        })
        .await;
        let began = std::time::Instant::now();
        assert_eq!(
            browser_socket_path(served.addr.port()).await.as_deref(),
            Some("/devtools/browser/kept-open")
        );
        assert!(began.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn it_refuses_what_is_not_a_devtools_socket() {
        assert_eq!(socket_path_in("{}"), None);
        assert_eq!(socket_path_in("no json"), None);
        assert_eq!(
            socket_path_in(r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:1/other"}"#),
            None
        );
    }
}
