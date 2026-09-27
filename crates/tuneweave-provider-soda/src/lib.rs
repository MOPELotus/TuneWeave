mod account;
mod authentication;
mod bdms;
mod client;
mod device;
mod identity;
mod library;
mod login;
pub mod media;
mod mfa;
mod provider;

pub use client::{SodaClient, SodaConfig};
pub use identity::SodaTrackIdentity;
pub use provider::SodaProvider;

#[cfg(test)]
mod test_http {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) async fn serve(
        responses: Vec<String>,
    ) -> (url::Url, tokio::task::JoinHandle<Vec<String>>) {
        serve_bytes(responses.into_iter().map(String::into_bytes).collect()).await
    }

    pub(crate) async fn serve_bytes(
        responses: Vec<Vec<u8>>,
    ) -> (url::Url, tokio::task::JoinHandle<Vec<String>>) {
        serve_inner(responses, None).await
    }

    pub(crate) struct PausedServer {
        pub origin: url::Url,
        pub arrived: tokio::sync::oneshot::Receiver<()>,
        pub release: tokio::sync::oneshot::Sender<()>,
        pub requests: tokio::task::JoinHandle<Vec<String>>,
    }

    pub(crate) async fn serve_paused(response: String) -> PausedServer {
        serve_paused_at(vec![response], 0).await
    }

    pub(crate) async fn serve_paused_at(responses: Vec<String>, index: usize) -> PausedServer {
        serve_bytes_paused_at(
            responses.into_iter().map(String::into_bytes).collect(),
            index,
        )
        .await
    }

    pub(crate) async fn serve_bytes_paused_at(
        responses: Vec<Vec<u8>>,
        index: usize,
    ) -> PausedServer {
        serve_bytes_paused_at_with_hold_timeout(
            responses,
            index,
            std::time::Duration::from_secs(10),
        )
        .await
    }

    pub(crate) async fn serve_bytes_paused_at_with_hold_timeout(
        responses: Vec<Vec<u8>>,
        index: usize,
        hold_timeout: std::time::Duration,
    ) -> PausedServer {
        assert!(index < responses.len());
        let (arrived_tx, arrived) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let (origin, requests) = serve_inner(
            responses,
            Some((index, arrived_tx, release_rx, hold_timeout)),
        )
        .await;
        PausedServer {
            origin,
            arrived,
            release,
            requests,
        }
    }

    async fn serve_inner(
        responses: Vec<Vec<u8>>,
        mut gate: Option<(
            usize,
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
            std::time::Duration,
        )>,
    ) -> (url::Url, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let origin =
            url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (index, response) in responses.into_iter().enumerate() {
                let (mut socket, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let size = socket.read(&mut buffer).await.unwrap();
                    assert!(size > 0, "connection closed before request headers");
                    bytes.extend_from_slice(&buffer[..size]);
                    assert!(bytes.len() <= 65536, "test request exceeds limit");
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                if gate
                    .as_ref()
                    .is_some_and(|(pause_at, _, _, _)| *pause_at == index)
                {
                    let (_, arrived, release, hold_timeout) = gate.take().unwrap();
                    arrived.send(()).unwrap();
                    tokio::time::timeout(hold_timeout, release)
                        .await
                        .unwrap()
                        .unwrap();
                }
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        (origin, task)
    }

    pub(crate) fn json(body: &str, cookie: Option<&str>) -> String {
        let cookie = cookie
            .map(|value| format!("Set-Cookie: {value}\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cookie}Connection: close\r\n\r\n{body}",
            body.len()
        )
    }
}
