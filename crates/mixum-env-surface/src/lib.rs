//! Surface transport shared by the container-side compositor and the host viewer.
//!
//! Wire format (little-endian):
//!
//! ```text
//! [0:4]  "MART"
//! [4]    version (1)
//! [5]    message type (1=surface create, 2=frame, 3=input, 4=surface destroy)
//! [6:10] payload length u32
//! [10:]  payload
//! ```
//!
//! A frame payload is a 33-byte header followed by PNG bytes:
//!
//! ```text
//! surface u32 | full_w u32 | full_h u32
//! x u32 | y u32 | w u32 | h u32
//! format u8 (0=cursor only, 2=png) | data_len u32 | data
//! ```

use serde::{Deserialize, Serialize};
use std::io::{self, Read};

pub const MAGIC: [u8; 4] = *b"MART";
pub const VERSION: u8 = 1;

pub const MSG_SURFACE_CREATE: u8 = 1;
pub const MSG_FRAME: u8 = 2;
pub const MSG_INPUT: u8 = 3;
pub const MSG_SURFACE_DESTROY: u8 = 4;

pub const FORMAT_NONE: u8 = 0;
pub const FORMAT_PNG: u8 = 2;

/// Maximum accepted message size (64 MiB); protects the viewer from bad peers.
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("bad magic/version")]
    BadHeader,
    #[error("message too large: {0}")]
    TooLarge(usize),
    #[error("truncated frame")]
    Truncated,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceInfo {
    pub id: u32,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub title: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceDestroy {
    pub id: u32,
}

/// Input event, host -> compositor (B1). Kept in the shared crate so both
/// sides agree on the wire names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    PointerMotion {
        surface: u32,
        x: f64,
        y: f64,
    },
    PointerButton {
        surface: u32,
        button: String,
        action: String,
    },
    Key {
        surface: u32,
        keysym: String,
        action: String,
    },
    Focus {
        surface: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub surface: u32,
    pub full_w: u32,
    pub full_h: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// 0 = cursor-only, 2 = PNG.
    pub format: u8,
    pub data: Vec<u8>,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(33 + self.data.len());
        for value in [
            self.surface,
            self.full_w,
            self.full_h,
            self.x,
            self.y,
            self.w,
            self.h,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.push(self.format);
        out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() < 33 {
            return Err(ProtocolError::Truncated);
        }
        let read_u32 = |offset: usize| {
            u32::from_le_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ])
        };
        let format = payload[28];
        let data_len = read_u32(29) as usize;
        if payload.len() < 33 + data_len {
            return Err(ProtocolError::Truncated);
        }
        Ok(Self {
            surface: read_u32(0),
            full_w: read_u32(4),
            full_h: read_u32(8),
            x: read_u32(12),
            y: read_u32(16),
            w: read_u32(20),
            h: read_u32(24),
            format,
            data: payload[33..33 + data_len].to_vec(),
        })
    }
}

pub fn encode_message(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + payload.len());
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn read_message<R: Read>(reader: &mut R) -> Result<(u8, Vec<u8>), ProtocolError> {
    let mut head = [0u8; 10];
    reader.read_exact(&mut head)?;
    if head[0..4] != MAGIC || head[4] != VERSION {
        return Err(ProtocolError::BadHeader);
    }
    let len = u32::from_le_bytes([head[6], head[7], head[8], head[9]]) as usize;
    if len > MAX_MESSAGE {
        return Err(ProtocolError::TooLarge(len));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok((head[5], payload))
}

pub fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    Ok(serde_json::to_vec(value)?)
}

pub fn decode_json<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T, ProtocolError> {
    Ok(serde_json::from_slice(payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_round_trip() {
        let frame = Frame {
            surface: 7,
            full_w: 1000,
            full_h: 700,
            x: 10,
            y: 20,
            w: 30,
            h: 40,
            format: FORMAT_PNG,
            data: vec![0x89, b'P', b'N', b'G'],
        };
        assert_eq!(Frame::decode(&frame.encode()).unwrap(), frame);
    }

    #[test]
    fn message_round_trip() {
        let msg = encode_message(MSG_FRAME, b"hello");
        let (kind, payload) = read_message(&mut Cursor::new(msg)).unwrap();
        assert_eq!(kind, MSG_FRAME);
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut msg = encode_message(MSG_FRAME, b"x");
        msg[0] = b'X';
        assert!(matches!(
            read_message(&mut Cursor::new(msg)),
            Err(ProtocolError::BadHeader)
        ));
    }
}
