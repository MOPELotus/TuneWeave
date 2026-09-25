use super::*;
use crate::{KugouConfig, account::tests::session};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

pub(crate) const SEED: &str = "a1B2c3";
pub(crate) struct Frame {
    pub(crate) bytes: Vec<u8>,
    pub(crate) gate: Option<oneshot::Receiver<()>>,
}
impl From<String> for Frame {
    fn from(value: String) -> Self {
        Self {
            bytes: value.into_bytes(),
            gate: None,
        }
    }
}
pub(crate) fn binary(status: u16, mime: &str, body: Vec<u8>) -> Frame {
    let mut bytes=format!("HTTP/1.1 {status} Test\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
    bytes.extend(body);
    Frame { bytes, gate: None }
}
pub(crate) fn encrypted(data: Value) -> Frame {
    let bytes = serde_json::to_vec(&json!({"status":1,"error_code":0,"data":data})).unwrap();
    binary(
        200,
        "application/octet-stream",
        Cipher { seed: SEED.into() }.encode(&bytes).unwrap(),
    )
}
#[derive(Clone)]
pub(crate) struct Request {
    pub(crate) head: String,
    pub(crate) body: Vec<u8>,
}
pub(crate) fn plaintext(request: &Request) -> Value {
    serde_json::from_slice(&Cipher { seed: SEED.into() }.decode(&request.body).unwrap()).unwrap()
}
pub(crate) struct Fixture {
    pub(crate) client: KugouClient,
    pub(crate) requests: tokio::task::JoinHandle<Vec<Request>>,
    pub(crate) seen: mpsc::UnboundedReceiver<Request>,
}
pub(crate) async fn server(frames: Vec<Frame>) -> Fixture {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (tx, seen) = mpsc::unbounded_channel();
    let requests = tokio::spawn(async move {
        let mut all = Vec::new();
        for frame in frames {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let request = loop {
                let mut buffer = [0; 8192];
                let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() < 8 * 1024 * 1024);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break Request {
                            head: head.into(),
                            body: bytes[end + 4..end + 4 + length].to_vec(),
                        };
                    }
                }
            };
            let _ = tx.send(request.clone());
            all.push(request);
            if let Some(gate) = frame.gate {
                let _ = gate.await;
            }
            let _ = socket.write_all(&frame.bytes).await;
            let _ = socket.shutdown().await;
        }
        all
    });
    let mut client = KugouClient::new(&KugouConfig::default()).unwrap();
    client.http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    client.login_test_origin = Some(origin);
    client.cloud_test_seed = Some(SEED.into());
    Fixture {
        client,
        requests,
        seen,
    }
}

#[test]
fn cloud_cipher_uses_independent_vector_and_never_accepts_plaintext_success() {
    let c = Cipher { seed: SEED.into() };
    let vector = BASE64
        .decode("VaEdhw4/84jD8Xt1LOFB21/sBr/kUU2u3lVldhSikl4=")
        .unwrap();
    assert_eq!(c.encode(br#"{"token":"deadbeef-token"}"#).unwrap(), vector);
    assert_eq!(c.decode(&vector).unwrap(), br#"{"token":"deadbeef-token"}"#);
    for bytes in [
        br#"{"status":1,"error_code":0,"data":{}}"#.as_slice(),
        b"garbage",
        &vector[..vector.len() - 1],
    ] {
        assert_eq!(c.decode(bytes).unwrap_err().code, ErrorCode::UpstreamError);
    }
    assert_eq!(
        c.decode(br#"{"status":0,"error_code":20017}"#)
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        let s = session(client);
        let p = c.portrait(&s).unwrap();
        assert_eq!(p.len(), 256);
        assert_eq!(p, p.to_ascii_uppercase());
        assert_ne!(p, c.portrait(&s).unwrap());
        let modulus =
            num_bigint::BigUint::parse_bytes(crypto::rsa_modulus(client).unwrap().as_bytes(), 16)
                .unwrap();
        assert!(num_bigint::BigUint::parse_bytes(p.as_bytes(), 16).unwrap() < modulus);
    }
}

#[tokio::test]
async fn cloud_protocol_posts_binary_orders_with_client_specific_keys_and_zero_based_positions() {
    for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for lists in [false, true] {
            let mut f=server(vec![encrypted(json!({"userid":123456789,"listid":37,"type":1,"list_ver":10,"pre_list_ver":9,"total_ver":10,"pre_total_ver":9}))]).await;
            let s = session(kind);
            let result = if lists {
                f.client
                    .native_reorder_lists(
                        &s,
                        9,
                        &[
                            ListPosition {
                                listid: 37,
                                kind: 0,
                                sort: 0,
                            },
                            ListPosition {
                                listid: 37,
                                kind: 1,
                                sort: 0,
                            },
                        ],
                    )
                    .await
            } else {
                f.client.native_reorder_files(&s, 37, 1, 9, &[83, 81]).await
            }
            .unwrap();
            assert_eq!(result.version, Some(10));
            assert_eq!(result.previous_version, Some(9));
            let request = f.seen.recv().await.unwrap();
            let path = request
                .head
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
            assert_eq!(
                url.path(),
                if lists {
                    "/v1/modify_list_sort"
                } else {
                    "/v1/modify_song_sort"
                }
            );
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(query.len(), 7);
            assert_eq!(query["appid"], kind.appid().to_string());
            let salt = if kind == KugouLoginClient::Standard {
                crate::signing::ANDROID_SALT
            } else {
                "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA"
            };
            assert_eq!(
                query["key"],
                format!(
                    "{:x}",
                    Md5::digest(format!(
                        "{}{salt}{}{}",
                        kind.appid(),
                        kind.clientver(),
                        query["clienttime"]
                    ))
                )
            );
            assert!(!request.head.contains(&s.token));
            assert!(
                request
                    .head
                    .contains("x-router: cloudlist.service.kugou.com")
            );
            let data = plaintext(&request);
            assert_eq!(data["data"][0]["sort"], 0);
            assert!(data.get("token").is_none());
            if lists {
                assert_eq!(data["total_ver"], 9);
                assert_eq!(data["data"][1]["type"], 1);
            } else {
                assert_eq!(data["list_ver"], 9);
                assert_eq!(data["type"], 1);
                assert_eq!(data["data"][0]["fileid"], 83);
            }
            assert_eq!(f.requests.await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn cloud_acknowledgements_reject_plain_success_bad_types_identity_and_transport_failures_without_retry()
 {
    let cases = vec![
        (
            binary(
                200,
                "application/json",
                br#"{"status":1,"error_code":0,"data":{}}"#.to_vec(),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            binary(
                200,
                "application/json",
                br#"{"status":0,"error_code":20017}"#.to_vec(),
            ),
            ErrorCode::AuthenticationRequired,
        ),
        (encrypted(json!({"userid":9})), ErrorCode::Conflict),
        (encrypted(json!({"listid":9})), ErrorCode::Conflict),
        (encrypted(json!({"type":0})), ErrorCode::Conflict),
        (
            encrypted(json!({"list_ver":"09"})),
            ErrorCode::UpstreamError,
        ),
        (encrypted(json!({"code":2})), ErrorCode::UpstreamError),
        (encrypted(json!([])), ErrorCode::UpstreamError),
        (
            binary(200, "text/html", vec![0; 16]),
            ErrorCode::UpstreamError,
        ),
        (
            binary(200, "application/octet-stream", vec![0; 1_048_577]),
            ErrorCode::UpstreamError,
        ),
        (
            binary(503, "application/json", vec![]),
            ErrorCode::UpstreamError,
        ),
    ];
    for (frame, code) in cases {
        let f = server(vec![frame]).await;
        let error = f
            .client
            .native_reorder_files(&session(KugouLoginClient::Standard), 37, 1, 9, &[83, 81])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, code);
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}
