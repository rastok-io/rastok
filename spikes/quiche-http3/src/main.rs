use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::net::SocketAddr;

use quiche::h3::NameValue;

const CERTIFICATE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/test.crt");
const PRIVATE_KEY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/certs/test.key");
const MAX_DATAGRAM_SIZE: usize = 1_350;
const MAX_FLIGHT_DATAGRAMS: usize = 256;
const MAX_PUMP_ROUNDS: usize = 256;
const MAX_EXCHANGE_ROUNDS: usize = 64;

type SpikeResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct SpikeFailure(String);

impl fmt::Display for SpikeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SpikeFailure {}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(SpikeFailure(message.into()))
}

#[derive(Debug)]
struct SpikeReport {
    datagrams: usize,
    client_sent_packets: usize,
    client_received_packets: usize,
    server_sent_packets: usize,
    server_received_packets: usize,
}

impl fmt::Display for SpikeReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SPIKE_OK alpn=h3 requests=2 reverse_replies=true datagrams={} client_packets={}/{} server_packets={}/{}",
            self.datagrams,
            self.client_sent_packets,
            self.client_received_packets,
            self.server_sent_packets,
            self.server_received_packets,
        )
    }
}

#[derive(Default)]
struct ObservedMessage {
    headers: Option<Vec<quiche::h3::Header>>,
    body: Vec<u8>,
    finished: bool,
}

struct Datagram {
    bytes: Vec<u8>,
    send: quiche::SendInfo,
}

struct QuicPair {
    client: quiche::Connection,
    server: quiche::Connection,
    datagrams: usize,
}

impl QuicPair {
    fn connect() -> SpikeResult<Self> {
        let client_addr: SocketAddr = "127.0.0.1:44330".parse()?;
        let server_addr: SocketAddr = "127.0.0.1:44331".parse()?;

        let mut client_config = transport_config()?;
        client_config.load_verify_locations_from_file(CERTIFICATE)?;
        client_config.verify_peer(true);

        let mut server_config = transport_config()?;
        server_config.load_cert_chain_from_pem_file(CERTIFICATE)?;
        server_config.load_priv_key_from_pem_file(PRIVATE_KEY)?;

        let client_id_bytes = [0x43; 16];
        let client_id = quiche::ConnectionId::from_ref(&client_id_bytes);
        let server_id_bytes = [0x53; 16];
        let server_id = quiche::ConnectionId::from_ref(&server_id_bytes);

        let client = quiche::connect(
            Some("localhost"),
            &client_id,
            client_addr,
            server_addr,
            &mut client_config,
        )?;
        let server = quiche::accept(
            &server_id,
            None,
            server_addr,
            client_addr,
            &mut server_config,
        )?;

        Ok(Self {
            client,
            server,
            datagrams: 0,
        })
    }

    fn handshake(&mut self) -> SpikeResult<()> {
        self.advance()?;

        if !self.client.is_established() || !self.server.is_established() {
            return Err(failure(
                "the in-memory QUIC flight became idle before TLS established",
            ));
        }
        if self.client.application_proto() != b"h3" || self.server.application_proto() != b"h3" {
            return Err(failure("TLS did not negotiate the h3 ALPN"));
        }
        if self.client.peer_cert().is_none() {
            return Err(failure("the client observed no server certificate"));
        }
        if self.server.server_name() != Some("localhost") {
            return Err(failure("the server observed the wrong SNI"));
        }

        Ok(())
    }

    fn advance(&mut self) -> SpikeResult<()> {
        for _ in 0..MAX_PUMP_ROUNDS {
            let client_flight = emit_flight(&mut self.client)?;
            let client_count = client_flight.len();
            deliver_flight(&mut self.server, client_flight)?;

            let server_flight = emit_flight(&mut self.server)?;
            let server_count = server_flight.len();
            deliver_flight(&mut self.client, server_flight)?;

            self.datagrams = self
                .datagrams
                .checked_add(client_count + server_count)
                .ok_or_else(|| failure("datagram counter overflow"))?;

            if client_count == 0 && server_count == 0 {
                return Ok(());
            }
        }

        Err(failure("the in-memory QUIC pump did not become idle"))
    }
}

fn transport_config() -> SpikeResult<quiche::Config> {
    let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION)?;
    config.set_application_protos(quiche::h3::APPLICATION_PROTOCOL)?;
    config.set_max_idle_timeout(10_000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(1_000_000);
    config.set_initial_max_stream_data_bidi_local(100_000);
    config.set_initial_max_stream_data_bidi_remote(100_000);
    config.set_initial_max_stream_data_uni(100_000);
    config.set_initial_max_streams_bidi(16);
    config.set_initial_max_streams_uni(16);
    config.set_disable_active_migration(true);
    Ok(config)
}

fn emit_flight(connection: &mut quiche::Connection) -> SpikeResult<Vec<Datagram>> {
    let mut flight = Vec::new();

    for _ in 0..MAX_FLIGHT_DATAGRAMS {
        let mut bytes = vec![0; 65_535];
        match connection.send(&mut bytes) {
            Ok((written, send)) => {
                bytes.truncate(written);
                flight.push(Datagram { bytes, send });
            }
            Err(quiche::Error::Done) => return Ok(flight),
            Err(error) => return Err(Box::new(error)),
        }
    }

    Err(failure("one QUIC flight exceeded its datagram budget"))
}

fn deliver_flight(connection: &mut quiche::Connection, flight: Vec<Datagram>) -> SpikeResult<()> {
    for mut datagram in flight {
        let received = connection.recv(
            &mut datagram.bytes,
            quiche::RecvInfo {
                from: datagram.send.from,
                to: datagram.send.to,
            },
        )?;
        if received != datagram.bytes.len() {
            return Err(failure("quiche only consumed part of one UDP datagram"));
        }
    }
    Ok(())
}

fn send_request(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    path: &[u8],
    body: &[u8],
) -> SpikeResult<u64> {
    let content_length = body.len().to_string();
    let headers = [
        quiche::h3::Header::new(b":method", b"POST"),
        quiche::h3::Header::new(b":scheme", b"https"),
        quiche::h3::Header::new(b":authority", b"localhost"),
        quiche::h3::Header::new(b":path", path),
        quiche::h3::Header::new(b"content-type", b"application/octet-stream"),
        quiche::h3::Header::new(b"content-length", content_length.as_bytes()),
    ];
    let stream = h3.send_request(connection, &headers, false)?;
    send_complete_body(connection, h3, stream, body)?;
    Ok(stream)
}

fn send_response(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    stream: u64,
    body: &[u8],
) -> SpikeResult<()> {
    let content_length = body.len().to_string();
    let headers = [
        quiche::h3::Header::new(b":status", b"200"),
        quiche::h3::Header::new(b"content-type", b"application/octet-stream"),
        quiche::h3::Header::new(b"content-length", content_length.as_bytes()),
    ];
    h3.send_response(connection, stream, &headers, false)?;
    send_complete_body(connection, h3, stream, body)
}

fn send_complete_body(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    stream: u64,
    body: &[u8],
) -> SpikeResult<()> {
    let written = h3.send_body(connection, stream, body, true)?;
    if written != body.len() {
        return Err(failure(format!(
            "the small spike body was flow-controlled after {written}/{} bytes",
            body.len(),
        )));
    }
    Ok(())
}

fn drain_body(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    stream: u64,
    destination: &mut Vec<u8>,
) -> SpikeResult<()> {
    let mut buffer = [0; 4_096];
    loop {
        match h3.recv_body(connection, stream, &mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => destination.extend_from_slice(&buffer[..read]),
            Err(quiche::h3::Error::Done) => return Ok(()),
            Err(error) => return Err(Box::new(error)),
        }
    }
}

fn poll_server(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    requests: &mut BTreeMap<u64, ObservedMessage>,
) -> SpikeResult<()> {
    loop {
        match h3.poll(connection) {
            Ok((stream, quiche::h3::Event::Headers { list, .. })) => {
                requests.entry(stream).or_default().headers = Some(list);
            }
            Ok((stream, quiche::h3::Event::Data)) => {
                let message = requests.entry(stream).or_default();
                drain_body(connection, h3, stream, &mut message.body)?;
            }
            Ok((stream, quiche::h3::Event::Finished)) => {
                requests.entry(stream).or_default().finished = true;
            }
            Ok((stream, quiche::h3::Event::Reset(code))) => {
                return Err(failure(format!(
                    "the client reset request stream {stream} with code {code}",
                )));
            }
            Ok((_stream, quiche::h3::Event::PriorityUpdate)) => {}
            Ok((identifier, quiche::h3::Event::GoAway)) => {
                return Err(failure(format!(
                    "the client sent GOAWAY {identifier} before the exchange completed",
                )));
            }
            Err(quiche::h3::Error::Done) => return Ok(()),
            Err(error) => return Err(Box::new(error)),
        }
    }
}

fn poll_client(
    connection: &mut quiche::Connection,
    h3: &mut quiche::h3::Connection,
    responses: &mut BTreeMap<u64, ObservedMessage>,
) -> SpikeResult<()> {
    loop {
        match h3.poll(connection) {
            Ok((stream, quiche::h3::Event::Headers { list, .. })) => {
                responses.entry(stream).or_default().headers = Some(list);
            }
            Ok((stream, quiche::h3::Event::Data)) => {
                let message = responses.entry(stream).or_default();
                drain_body(connection, h3, stream, &mut message.body)?;
            }
            Ok((stream, quiche::h3::Event::Finished)) => {
                responses.entry(stream).or_default().finished = true;
            }
            Ok((stream, quiche::h3::Event::Reset(code))) => {
                return Err(failure(format!(
                    "the server reset response stream {stream} with code {code}",
                )));
            }
            Ok((_stream, quiche::h3::Event::PriorityUpdate)) => {}
            Ok((identifier, quiche::h3::Event::GoAway)) => {
                return Err(failure(format!(
                    "the server sent GOAWAY {identifier} before the exchange completed",
                )));
            }
            Err(quiche::h3::Error::Done) => return Ok(()),
            Err(error) => return Err(Box::new(error)),
        }
    }
}

fn header_value<'a>(headers: &'a [quiche::h3::Header], name: &[u8]) -> Option<&'a [u8]> {
    headers
        .iter()
        .find(|header| header.name() == name)
        .map(NameValue::value)
}

fn response_for_request(message: &ObservedMessage) -> SpikeResult<&'static [u8]> {
    let headers = message
        .headers
        .as_deref()
        .ok_or_else(|| failure("a finished request had no headers"))?;
    if header_value(headers, b":method") != Some(b"POST") {
        return Err(failure("the server observed a non-POST request"));
    }

    match (header_value(headers, b":path"), message.body.as_slice()) {
        (Some(b"/alpha"), b"alpha-input") => Ok(b"alpha-response"),
        (Some(b"/beta"), b"beta-input") => Ok(b"beta-response"),
        _ => Err(failure(
            "the server observed a crossed or malformed request",
        )),
    }
}

fn verify_response(message: &ObservedMessage, expected: &[u8]) -> SpikeResult<()> {
    if !message.finished {
        return Err(failure("an HTTP/3 response never reached FIN"));
    }
    let headers = message
        .headers
        .as_deref()
        .ok_or_else(|| failure("an HTTP/3 response had no headers"))?;
    if header_value(headers, b":status") != Some(b"200") {
        return Err(failure("an HTTP/3 response did not carry status 200"));
    }
    if message.body != expected {
        return Err(failure("concurrent HTTP/3 responses crossed streams"));
    }
    Ok(())
}

fn exercise() -> SpikeResult<SpikeReport> {
    let mut pair = QuicPair::connect()?;
    pair.handshake()?;

    let h3_config = quiche::h3::Config::new()?;
    let mut client_h3 = quiche::h3::Connection::with_transport(&mut pair.client, &h3_config)?;
    let mut server_h3 = quiche::h3::Connection::with_transport(&mut pair.server, &h3_config)?;

    let alpha_stream = send_request(&mut pair.client, &mut client_h3, b"/alpha", b"alpha-input")?;
    let beta_stream = send_request(&mut pair.client, &mut client_h3, b"/beta", b"beta-input")?;
    if alpha_stream == beta_stream {
        return Err(failure("two requests reused one HTTP/3 stream"));
    }

    let expected = BTreeMap::from([
        (alpha_stream, b"alpha-response".as_slice()),
        (beta_stream, b"beta-response".as_slice()),
    ]);
    let mut requests = BTreeMap::new();
    let mut responses = BTreeMap::new();
    let mut replies_sent = false;

    for _ in 0..MAX_EXCHANGE_ROUNDS {
        pair.advance()?;
        poll_server(&mut pair.server, &mut server_h3, &mut requests)?;

        if !replies_sent
            && requests.len() == expected.len()
            && requests.values().all(|message| message.finished)
        {
            let mut replies = Vec::new();
            for (stream, request) in requests.iter().rev() {
                replies.push((*stream, response_for_request(request)?));
            }
            for (stream, body) in replies {
                send_response(&mut pair.server, &mut server_h3, stream, body)?;
            }
            replies_sent = true;
        }

        poll_client(&mut pair.client, &mut client_h3, &mut responses)?;
        if responses.len() == expected.len() && responses.values().all(|message| message.finished) {
            break;
        }
    }

    if !replies_sent {
        return Err(failure("the server never observed both complete requests"));
    }
    for (stream, body) in expected {
        let response = responses
            .get(&stream)
            .ok_or_else(|| failure(format!("stream {stream} had no response")))?;
        verify_response(response, body)?;
    }

    pair.advance()?;
    let client_stats = pair.client.stats();
    let server_stats = pair.server.stats();
    Ok(SpikeReport {
        datagrams: pair.datagrams,
        client_sent_packets: client_stats.sent,
        client_received_packets: client_stats.recv,
        server_sent_packets: server_stats.sent,
        server_received_packets: server_stats.recv,
    })
}

fn main() {
    match exercise() {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("SPIKE_FAIL {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_concurrent_streams_keep_their_responses() {
        let result = super::exercise();
        assert!(result.is_ok(), "{result:?}");
    }
}
