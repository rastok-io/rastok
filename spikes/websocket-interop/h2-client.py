"""Independent RFC 8441 client: hyper-h2 transport plus wsproto framing."""

from __future__ import annotations

import socket
import sys
import time

import h2.config
import h2.connection
import h2.events
import h2.settings
from wsproto.frame_protocol import FrameProtocol, Opcode


def receive(connection: h2.connection.H2Connection, stream: socket.socket):
    data = stream.recv(65_535)
    if not data:
        raise RuntimeError("the HTTP/2 peer closed before the proof completed")
    events = connection.receive_data(data)
    pending = connection.data_to_send()
    if pending:
        stream.sendall(pending)
    return events


def main() -> None:
    if len(sys.argv) != 5:
        raise SystemExit("usage: h2-client.py HOST PORT PATH MESSAGE")
    host, port_text, path, expected = sys.argv[1:]
    port = int(port_text)
    stream = socket.create_connection((host, port), timeout=5)
    stream.settimeout(5)
    connection = h2.connection.H2Connection(
        config=h2.config.H2Configuration(client_side=True, header_encoding="utf-8")
    )
    connection.initiate_connection()
    connection.update_settings(
        {h2.settings.SettingCodes.ENABLE_CONNECT_PROTOCOL: 1}
    )
    stream.sendall(connection.data_to_send())

    deadline = time.monotonic() + 5
    connect_enabled = False
    while not connect_enabled:
        for event in receive(connection, stream):
            if isinstance(event, h2.events.RemoteSettingsChanged):
                setting = event.changed_settings.get(
                    h2.settings.SettingCodes.ENABLE_CONNECT_PROTOCOL
                )
                connect_enabled = setting is not None and setting.new_value == 1
        if time.monotonic() >= deadline:
            raise RuntimeError("the server never enabled RFC 8441 CONNECT")

    stream_id = connection.get_next_available_stream_id()
    connection.send_headers(
        stream_id,
        [
            (":method", "CONNECT"),
            (":scheme", "http"),
            (":authority", f"{host}:{port}"),
            (":path", path),
            (":protocol", "websocket"),
            ("sec-websocket-version", "13"),
        ],
        end_stream=False,
    )
    stream.sendall(connection.data_to_send())
    accepted = False
    while not accepted:
        for event in receive(connection, stream):
            if isinstance(event, h2.events.ResponseReceived) and event.stream_id == stream_id:
                headers = dict(event.headers)
                if headers.get(":status") != "200":
                    raise RuntimeError(f"unexpected Extended CONNECT response {headers!r}")
                accepted = True

    websocket = FrameProtocol(client=True, extensions=[])
    frame = websocket.send_data(expected)
    connection.send_data(stream_id, frame, end_stream=False)
    stream.sendall(connection.data_to_send())
    reply = ""
    complete = False
    while not complete:
        for event in receive(connection, stream):
            if isinstance(event, h2.events.DataReceived) and event.stream_id == stream_id:
                connection.acknowledge_received_data(
                    event.flow_controlled_length, stream_id
                )
                websocket.receive_bytes(event.data)
                for message in websocket.received_frames():
                    if message.opcode is Opcode.TEXT:
                        reply += str(message.payload)
                        complete = message.message_finished
        stream.sendall(connection.data_to_send())
    if reply != expected:
        raise RuntimeError(f"crossed or changed reply: {reply!r}")

    close = websocket.close(code=1000, reason="proved")
    connection.send_data(stream_id, close, end_stream=False)
    stream.sendall(connection.data_to_send())
    print("H2_EXTENDED_CONNECT_OK")


if __name__ == "__main__":
    main()
