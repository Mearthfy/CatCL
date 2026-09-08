use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 3] = *b"CCL";
pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_CONTROL_FRAME: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamHandshake {
    pub magic: [u8; 3],
    pub protocol_version: u16,
    pub session_id: [u8; 16],
    pub peer_id: [u8; 16],
    pub nonce: [u8; 32],
}

impl StreamHandshake {
    pub fn new(session_id: [u8; 16], peer_id: [u8; 16], nonce: [u8; 32]) -> Self {
        Self {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            session_id,
            peer_id,
            nonce,
        }
    }
}
