#![allow(clippy::expect_used)]

use std::convert::Infallible;
use std::error::Error;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{self, Message};

type EvidenceResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostedHttpProbe {
    Http1,
    Http2,
    Http3Configuration,
}

fn runtime() -> EvidenceResult<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

async fn echo_http(
    request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let version = request.version();
    let body = request.into_body().collect().await?.to_bytes();
    let version = match version {
        Version::HTTP_11 => b"h1:".as_slice(),
        Version::HTTP_2 => b"h2:".as_slice(),
        _ => b"other:".as_slice(),
    };
    let mut response = version.to_vec();
    response.extend_from_slice(&body);
    Ok(Response::new(Full::new(Bytes::from(response))))
}

async fn exercise_http1() -> EvidenceResult<Vec<u8>> {
    let (client_io, server_io) = tokio::io::duplex(8 * 1024);
    let server = tokio::spawn(async move {
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service_fn(echo_http))
            .await
    });
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(client_io)).await?;
    let client = tokio::spawn(connection);
    let request = Request::builder()
        .method("POST")
        .uri("http://localhost/probe")
        .body(Full::new(Bytes::from_static(b"payload")))?;
    let body = sender
        .send_request(request)
        .await?
        .into_body()
        .collect()
        .await?;
    drop(sender);
    client.await??;
    server.await??;
    Ok(body.to_bytes().to_vec())
}

async fn exercise_http2() -> EvidenceResult<Vec<u8>> {
    let (client_io, server_io) = tokio::io::duplex(8 * 1024);
    let server = tokio::spawn(async move {
        hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(server_io), service_fn(echo_http))
            .await
    });
    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake(TokioIo::new(client_io))
        .await?;
    let client = tokio::spawn(connection);
    let request = Request::builder()
        .method("POST")
        .uri("https://localhost/probe")
        .body(Full::new(Bytes::from_static(b"payload")))?;
    let body = sender
        .send_request(request)
        .await?
        .into_body()
        .collect()
        .await?;
    drop(sender);
    client.await??;
    server.await??;
    Ok(body.to_bytes().to_vec())
}

async fn exercise_hosted_http_probe(
    probe: HostedHttpProbe,
    limits: super::pump::EngineLimits,
) -> Result<Vec<u8>, String> {
    match probe {
        HostedHttpProbe::Http1 => exercise_http1().await.map_err(|error| error.to_string()),
        HostedHttpProbe::Http2 => exercise_http2().await.map_err(|error| error.to_string()),
        HostedHttpProbe::Http3Configuration => super::tls::quiche_client(None, None, limits)
            .map(|_| b"h3:configured".to_vec())
            .map_err(|error| error.to_string()),
    }
}

fn exercise_http_inside_the_hosted_pump() -> EvidenceResult<Vec<(HostedHttpProbe, Vec<u8>)>> {
    let limits = super::pump::EngineLimits::new(8, 2, 8, 64 * 1024)?;
    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    let mut engine = super::pump::HostedNetworkEngine::<HostedHttpProbe, (), _>::start(
        "rastok-network-http-evidence",
        limits,
        move |mut runtime| async move {
            while let Some(input) = runtime.recv().await {
                let super::pump::EngineInput::Work(work) = input else {
                    break;
                };
                let probe = *work.value();
                drop(work);
                let result = exercise_hosted_http_probe(probe, limits).await;
                let bytes = result.as_ref().map_or_else(|error| error.len(), Vec::len);
                if runtime.emit((probe, result), bytes).await.is_err() {
                    break;
                }
                if completed_tx.send(()).is_err() {
                    break;
                }
            }
        },
    )?;
    for probe in [
        HostedHttpProbe::Http1,
        HostedHttpProbe::Http2,
        HostedHttpProbe::Http3Configuration,
    ] {
        engine
            .try_submit(probe, 1)
            .map_err(|_| std::io::Error::other("the hosted HTTP work queue refused its probe"))?;
    }
    for _ in 0..3 {
        completed_rx.recv_timeout(Duration::from_secs(5))?;
    }
    let batch = engine.drain_events(3);
    if batch.events.len() != 3 {
        return Err(std::io::Error::other("the hosted HTTP pump lost an event").into());
    }
    batch
        .events
        .into_iter()
        .map(|(probe, result)| {
            result
                .map(|bytes| (probe, bytes))
                .map_err(|message| std::io::Error::other(message).into())
        })
        .collect()
}

async fn exercise_synthetic_websocket() -> EvidenceResult<()> {
    use tungstenite::protocol::frame::Frame;
    use tungstenite::protocol::frame::coding::{Data as FrameData, OpCode};

    let config = super::websocket_codec::bounded_config(4 * 1024)?;
    let (client_io, server_io) = tokio::io::duplex(8 * 1024);
    let mut client = tokio_tungstenite::WebSocketStream::from_raw_socket(
        client_io,
        tungstenite::protocol::Role::Client,
        Some(config),
    )
    .await;
    let mut server = tokio_tungstenite::WebSocketStream::from_raw_socket(
        server_io,
        tungstenite::protocol::Role::Server,
        Some(super::websocket_codec::bounded_config(4 * 1024)?),
    )
    .await;

    let server_task = tokio::spawn(async move {
        let mut messages = 0usize;
        let mut ping = false;
        while messages < 3 || !ping {
            let message = server
                .next()
                .await
                .ok_or("the synthetic WebSocket ended before three complete messages")??;
            match message {
                Message::Text(_) | Message::Binary(_) => {
                    messages = messages.saturating_add(1);
                    server.send(message).await?;
                }
                Message::Ping(payload) => {
                    ping = true;
                    server.send(Message::Pong(payload)).await?;
                }
                Message::Pong(_) | Message::Close(_) | Message::Frame(_) => {}
            }
        }
        EvidenceResult::Ok(())
    });
    client.send(Message::Text("hello".into())).await?;
    client
        .send(Message::Binary(Bytes::from_static(b"bytes")))
        .await?;
    client
        .send(Message::Frame(Frame::message(
            Bytes::from_static(b"frag"),
            OpCode::Data(FrameData::Text),
            false,
        )))
        .await?;
    client
        .send(Message::Frame(Frame::message(
            Bytes::from_static(b"mented"),
            OpCode::Data(FrameData::Continue),
            true,
        )))
        .await?;
    client
        .send(Message::Ping(Bytes::from_static(b"probe")))
        .await?;
    assert_eq!(
        client.next().await.transpose()?,
        Some(Message::Text("hello".into()))
    );
    assert_eq!(
        client.next().await.transpose()?,
        Some(Message::Binary(Bytes::from_static(b"bytes")))
    );
    assert_eq!(
        client.next().await.transpose()?,
        Some(Message::Text("fragmented".into()))
    );
    assert_eq!(
        client.next().await.transpose()?,
        Some(Message::Pong(Bytes::from_static(b"probe")))
    );
    server_task.await??;
    Ok(())
}

async fn websocket_upgrade_response(
    mut request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let Some(key) = request
        .headers()
        .get(hyper::header::SEC_WEBSOCKET_KEY)
        .cloned()
    else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::new()))
            .expect("the fixed HTTP response is valid"));
    };
    let upgrade = hyper::upgrade::on(&mut request);
    let _session = tokio::spawn(async move {
        let Ok(upgraded) = upgrade.await else {
            return;
        };
        let Ok(config) = super::websocket_codec::bounded_config(4 * 1024) else {
            return;
        };
        let mut websocket = tokio_tungstenite::WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            tungstenite::protocol::Role::Server,
            Some(config),
        )
        .await;
        if let Some(Ok(message)) = websocket.next().await {
            let _ = websocket.send(message).await;
        }
    });

    Ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::CONNECTION, "upgrade")
        .header(hyper::header::UPGRADE, "websocket")
        .header(
            hyper::header::SEC_WEBSOCKET_ACCEPT,
            tungstenite::handshake::derive_accept_key(key.as_bytes()),
        )
        .body(Full::new(Bytes::new()))
        .expect("the fixed WebSocket upgrade response is valid"))
}

async fn exercise_http1_websocket_upgrade() -> EvidenceResult<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await?;
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service_fn(websocket_upgrade_response))
            .with_upgrades()
            .await?;
        EvidenceResult::Ok(())
    });

    let stream = TcpStream::connect(address).await?;
    let (mut client, response) = tokio_tungstenite::client_async_with_config(
        format!("ws://{address}/socket"),
        stream,
        Some(super::websocket_codec::bounded_config(4 * 1024)?),
    )
    .await?;
    if response.status() != StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("unexpected WebSocket status {}", response.status()).into());
    }
    client.send(Message::Text("through-hyper".into())).await?;
    if client.next().await.transpose()? != Some(Message::Text("through-hyper".into())) {
        return Err("the HTTP/1.1 WebSocket echo changed the message".into());
    }
    client.close(None).await?;
    server.await??;
    Ok(())
}

#[test]
fn hyper_h1_h2_and_quiche_h3_coexist_in_one_hosted_binary() {
    assert_eq!(
        exercise_http_inside_the_hosted_pump()
            .expect("all three selected HTTP engines must coexist"),
        [
            (HostedHttpProbe::Http1, b"h1:payload".to_vec()),
            (HostedHttpProbe::Http2, b"h2:payload".to_vec()),
            (
                HostedHttpProbe::Http3Configuration,
                b"h3:configured".to_vec()
            ),
        ]
    );
}

#[test]
fn tungstenite_is_bounded_on_synthetic_and_http1_streams() {
    runtime()
        .expect("the evidence runtime must build")
        .block_on(async {
            exercise_synthetic_websocket().await?;
            exercise_http1_websocket_upgrade().await
        })
        .expect("the two proved WebSocket transports must pass");
}
