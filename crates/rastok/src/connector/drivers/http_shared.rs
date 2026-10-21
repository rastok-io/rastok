//! Protocol-neutral HTTP values shared by the client and server Drivers.

use crate::envelope::Data;
use crate::value::Value;

/// One HTTP version selected from a contract's ordered preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HttpVersion {
    Http1,
    Http2,
    Http3,
}

impl HttpVersion {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "http/1.1" => Some(Self::Http1),
            "http/2" => Some(Self::Http2),
            "http/3" => Some(Self::Http3),
            _ => None,
        }
    }

    pub(crate) const fn observation(self) -> &'static str {
        match self {
            Self::Http1 => "http/1.1",
            Self::Http2 => "http/2",
            Self::Http3 => "http/3",
        }
    }
}

/// Decodes one complete HTTP body under the sheet-selected logical codec.
pub(crate) fn decode_body(codec: &str, body: Vec<u8>) -> Option<Data> {
    match codec {
        "bytes" => Some(Data::materialized(Value::Bytes(body))),
        "text" => String::from_utf8(body)
            .ok()
            .map(Value::Text)
            .map(Data::materialized),
        "json" => {
            let text = String::from_utf8(body).ok()?;
            let (value, used) = rastok_core::json::parse_value(&text).ok()?;
            text.get(used..)?
                .chars()
                .all(|character| character.is_ascii_whitespace())
                .then_some(Data::materialized(value))
        }
        _ => None,
    }
}
