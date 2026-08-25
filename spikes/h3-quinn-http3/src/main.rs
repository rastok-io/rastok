use bytes::{Buf, Bytes};
use h3::error::ConnectionError;
use h3_quinn::quinn;
use http::{Request, Response, StatusCode};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::error::Error;
use std::fmt;
use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

const CA_CERTIFICATE: &[u8] = include_bytes!("../certs/ca.crt");
const SERVER_CERTIFICATE: &[u8] = include_bytes!("../certs/server.crt");
const SERVER_PRIVATE_KEY: &[u8] = include_bytes!("../certs/server.key");
const ALPN: &[u8] = b"h3";
const DEADLINE: Duration = Duration::from_secs(10);

type SpikeResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug)]
struct SpikeFailure(String);

impl fmt::Display for SpikeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SpikeFailure {}

fn failure(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(SpikeFailure(message.into()))
}

fn certificate(pem: &'static [u8]) -> SpikeResult<CertificateDer<'static>> {
    let mut reader = Cursor::new(pem);
    rustls_pemfile::certs(&mut reader)
        .next()
        .transpose()?
        .ok_or_else(|| failure("the fixture contains no certificate"))
}

fn fixture_private_key() -> SpikeResult<PrivateKeyDer<'static>> {
    let mut reader = Cursor::new(SERVER_PRIVATE_KEY);
    rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| failure("the fixture contains no private key"))
}

fn server_config() -> SpikeResult<quinn::ServerConfig> {
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate(SERVER_CERTIFICATE)?],
            fixture_private_key()?,
        )?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.max_early_data_size = 0;
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(crypto)))
}

fn client_config() -> SpikeResult<quinn::ClientConfig> {
    let mut roots =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots.add(certificate(CA_CERTIFICATE)?)?;
    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.enable_early_data = false;
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    Ok(quinn::ClientConfig::new(Arc::new(crypto)))
}

async fn serve(endpoint: quinn::Endpoint) -> SpikeResult<(Vec<String>, Vec<String>)> {
    let incoming = endpoint
        .accept()
        .await
        .ok_or_else(|| failure("the server endpoint closed before a connection arrived"))?;
    let connection = incoming.await?;
    let mut h3 = h3::server::Connection::new(h3_quinn::Connection::new(connection)).await?;
    let mut requests = Vec::new();
    for _ in 0..2 {
        let resolver = h3
            .accept()
            .await?
            .ok_or_else(|| failure("the client sent GOAWAY before both requests arrived"))?;
        let (request, stream) = resolver.resolve_request().await?;
        requests.push((request.uri().path().to_string(), stream));
    }

    let accepted_order = requests.iter().map(|(path, _)| path.clone()).collect();
    let mut completion_order = Vec::new();
    while let Some((path, mut stream)) = requests.pop() {
        let response = Response::builder().status(StatusCode::OK).body(())?;
        stream.send_response(response).await?;
        stream
            .send_data(Bytes::copy_from_slice(path.as_bytes()))
            .await?;
        stream.finish().await?;
        completion_order.push(path);
    }
    h3.shutdown(0).await?;
    endpoint.wait_idle().await;
    Ok((accepted_order, completion_order))
}

async fn request(
    mut sender: h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>,
    path: &'static str,
) -> SpikeResult<(String, String)> {
    let request = Request::builder()
        .method("POST")
        .uri(format!("https://localhost{path}"))
        .body(())?;
    let mut stream = sender.send_request(request).await?;
    stream.finish().await?;
    let response = stream.recv_response().await?;
    if response.status() != StatusCode::OK {
        return Err(failure(format!(
            "unexpected response status {}",
            response.status()
        )));
    }
    let mut body = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        while chunk.has_remaining() {
            let bytes = chunk.chunk();
            body.extend_from_slice(bytes);
            let len = bytes.len();
            chunk.advance(len);
        }
    }
    Ok((
        path.to_string(),
        String::from_utf8_lossy(&body).into_owned(),
    ))
}

async fn run() -> SpikeResult<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| failure("the Rustls ring provider was already installed differently"))?;

    let server = quinn::Endpoint::server(
        server_config()?,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    )?;
    let server_addr = server.local_addr()?;
    let server_task = tokio::spawn(serve(server));

    let mut client = quinn::Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
    client.set_default_client_config(client_config()?);
    let connection = client.connect(server_addr, "localhost")?.await?;
    let negotiated = connection
        .handshake_data()
        .and_then(|value| {
            value
                .downcast::<quinn::crypto::rustls::HandshakeData>()
                .ok()
        })
        .and_then(|data| data.protocol.clone())
        .ok_or_else(|| failure("the QUIC handshake reported no ALPN"))?;
    if negotiated.as_slice() != ALPN {
        return Err(failure("the QUIC handshake did not negotiate h3"));
    }

    let (mut driver, sender) = h3::client::new(h3_quinn::Connection::new(connection)).await?;
    let driver_task = tokio::spawn(async move {
        Err::<(), ConnectionError>(std::future::poll_fn(|cx| driver.poll_close(cx)).await)
    });
    let first = tokio::spawn(request(sender.clone(), "/first"));
    let second = tokio::spawn(request(sender.clone(), "/second"));
    drop(sender);

    let (first, second, accepted_order, completion_order) = tokio::time::timeout(DEADLINE, async {
        let first = first.await??;
        let second = second.await??;
        let (accepted_order, completion_order) = server_task.await??;
        SpikeResult::Ok((first, second, accepted_order, completion_order))
    })
    .await
    .map_err(|_| failure("the h3/Quinn comparison exceeded its deadline"))??;

    if first.0 != first.1 || second.0 != second.1 {
        return Err(failure("HTTP/3 response bodies crossed request streams"));
    }
    let reversed_acceptance: Vec<_> = accepted_order.into_iter().rev().collect();
    if completion_order != reversed_acceptance {
        return Err(failure(
            "the server did not complete requests in reverse order",
        ));
    }

    let _ = tokio::time::timeout(DEADLINE, driver_task).await;
    client.wait_idle().await;
    println!(
        "SPIKE_OK backend=h3-quinn udp=real alpn=h3 roots=public+fixture requests=2 reverse_replies=true goaway=true reclaimed=true"
    );
    Ok(())
}

async fn run_public_interop() -> SpikeResult<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| failure("the Rustls ring provider was already installed differently"))?;
    let server_addr = tokio::net::lookup_host(("cloudflare-quic.com", 443))
        .await?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| failure("public interoperability DNS returned no IPv4 address"))?;
    let mut endpoint =
        quinn::Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
    endpoint.set_default_client_config(client_config()?);
    let connection = endpoint
        .connect(server_addr, "cloudflare-quic.com")?
        .await?;
    let peer = connection
        .handshake_data()
        .and_then(|value| {
            value
                .downcast::<quinn::crypto::rustls::HandshakeData>()
                .ok()
        })
        .and_then(|data| data.protocol.clone())
        .ok_or_else(|| failure("the public peer reported no ALPN"))?;
    if peer.as_slice() != ALPN {
        return Err(failure("the public peer did not negotiate h3"));
    }

    let (mut driver, mut sender) =
        h3::client::new(h3_quinn::Connection::new(connection.clone())).await?;
    let driver_task = tokio::spawn(async move {
        Err::<(), ConnectionError>(std::future::poll_fn(|cx| driver.poll_close(cx)).await)
    });
    let request = Request::builder()
        .method("GET")
        .uri("https://cloudflare-quic.com/")
        .body(())?;
    let mut stream = sender.send_request(request).await?;
    stream.finish().await?;
    let response = tokio::time::timeout(DEADLINE, stream.recv_response())
        .await
        .map_err(|_| failure("the public HTTP/3 peer did not answer before the deadline"))??;
    if response.status().as_u16() >= 500 {
        return Err(failure(format!(
            "the public HTTP/3 peer returned {}",
            response.status()
        )));
    }
    while stream.recv_data().await?.is_some() {}
    connection.close(0u32.into(), b"interop complete");
    driver_task.abort();
    endpoint.wait_idle().await;
    println!(
        "SPIKE_OK backend=h3-quinn peer=cloudflare-quiche tls=verified alpn=h3 status={}",
        response.status()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> SpikeResult<()> {
    if std::env::args().any(|argument| argument == "--public") {
        run_public_interop().await
    } else {
        run().await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn comparison_completes() {
        let runtime = tokio::runtime::Runtime::new().expect("the test runtime must build");
        runtime
            .block_on(super::run())
            .expect("the h3/Quinn comparison must pass");
    }
}
