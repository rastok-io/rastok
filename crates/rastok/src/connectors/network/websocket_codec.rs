use std::error::Error;
use std::fmt;

const IO_BUFFER_BYTES: usize = 4 * 1024;
const FRAME_OVERHEAD_ALLOWANCE: usize = 1024;

#[derive(Debug)]
pub(super) struct WebSocketLimitError(&'static str);

impl fmt::Display for WebSocketLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for WebSocketLimitError {}

pub(super) fn bounded_config(
    max_message_bytes: usize,
) -> Result<tungstenite::protocol::WebSocketConfig, WebSocketLimitError> {
    if max_message_bytes == 0 {
        return Err(WebSocketLimitError(
            "the WebSocket message limit must be positive",
        ));
    }
    let max_write_buffer_size = max_message_bytes
        .checked_add(FRAME_OVERHEAD_ALLOWANCE)
        .ok_or(WebSocketLimitError(
            "the WebSocket write-buffer bound is not representable",
        ))?;
    Ok(tungstenite::protocol::WebSocketConfig::default()
        .read_buffer_size(IO_BUFFER_BYTES.min(max_message_bytes))
        .write_buffer_size(0)
        .max_write_buffer_size(max_write_buffer_size)
        .max_message_size(Some(max_message_bytes))
        .max_frame_size(Some(max_message_bytes))
        .accept_unmasked_frames(false))
}
