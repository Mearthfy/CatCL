use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{
    error::{P2pError, Result},
    protocol::{MAGIC, PROTOCOL_VERSION},
};

const PREFIX: &str = "CCL://";
const MAX_INVITE_BYTES: usize = 64 * 1024;
const CLOCK_SKEW_SECS: u64 = 120;
const MAX_CANDIDATES: usize = 32;
const MAX_PUBLIC_KEY_BYTES: usize = 16 * 1024;

#[derive(Serialize, Deserialize)]
struct Envelope {
    payload: Vec<u8>,
    checksum: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub address: String,
    pub kind: CandidateKind,
}

impl Candidate {
    pub fn socket_addr(&self) -> Result<SocketAddr> {
        self.address
            .parse()
            .map_err(|error| P2pError::InviteInvalid(format!("候选地址无效：{error}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateKind {
    Loopback,
    Local,
    Ipv6Direct,
    Ipv4Direct,
    Mapped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Offer {
    pub magic: [u8; 3],
    pub protocol_version: u16,
    pub session_id: [u8; 16],
    pub host_public_key: Vec<u8>,
    pub candidates: Vec<Candidate>,
    pub expires_at: u64,
    pub nonce: [u8; 32],
    pub capabilities: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Answer {
    pub magic: [u8; 3],
    pub protocol_version: u16,
    pub session_id: [u8; 16],
    pub peer_id: [u8; 16],
    pub offer_nonce: [u8; 32],
    pub peer_nonce: [u8; 32],
    pub candidates: Vec<Candidate>,
    pub expires_at: u64,
}

fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|error| P2pError::InviteInvalid(error.to_string()))
}

fn encode<T: Serialize>(value: &T) -> Result<String> {
    let payload = bincode::serialize(value)?;
    let envelope = Envelope {
        checksum: *blake3::hash(&payload).as_bytes(),
        payload,
    };
    let serialized = bincode::serialize(&envelope)?;
    let compressed = zstd::bulk::compress(&serialized, 3)
        .map_err(|error| P2pError::InviteInvalid(error.to_string()))?;
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(compressed)))
}

fn decode<T: DeserializeOwned>(code: &str) -> Result<T> {
    let body = code
        .trim()
        .strip_prefix(PREFIX)
        .ok_or_else(|| P2pError::InviteInvalid("缺少 CCL:// 标记".into()))?;
    let compressed = URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|error| P2pError::InviteInvalid(error.to_string()))?;
    if compressed.len() > MAX_INVITE_BYTES {
        return Err(P2pError::InviteInvalid("邀请数据过大".into()));
    }
    let bytes = zstd::bulk::decompress(&compressed, MAX_INVITE_BYTES)
        .map_err(|error| P2pError::InviteInvalid(error.to_string()))?;
    let envelope: Envelope = bincode::deserialize(&bytes)?;
    if blake3::hash(&envelope.payload).as_bytes() != &envelope.checksum {
        return Err(P2pError::InviteInvalid("校验和不匹配".into()));
    }
    bincode::deserialize(&envelope.payload).map_err(Into::into)
}

fn validate(magic: [u8; 3], version: u16, expires_at: u64) -> Result<()> {
    if magic != MAGIC {
        return Err(P2pError::InviteInvalid("magic 不匹配".into()));
    }
    if version != PROTOCOL_VERSION {
        return Err(P2pError::ProtocolMismatch {
            expected: PROTOCOL_VERSION,
            actual: version,
        });
    }
    if now()?.saturating_sub(CLOCK_SKEW_SECS) > expires_at {
        return Err(P2pError::InviteExpired);
    }
    Ok(())
}

impl Offer {
    pub fn encode(&self) -> Result<String> {
        encode(self)
    }
    pub fn decode(code: &str) -> Result<Self> {
        let value: Self = decode(code)?;
        validate(value.magic, value.protocol_version, value.expires_at)?;
        if value.host_public_key.is_empty() || value.host_public_key.len() > MAX_PUBLIC_KEY_BYTES {
            return Err(P2pError::InviteInvalid("主机证书大小无效".into()));
        }
        validate_candidates(&value.candidates)?;
        Ok(value)
    }
}

impl Answer {
    pub fn encode(&self) -> Result<String> {
        encode(self)
    }
    pub fn decode(code: &str) -> Result<Self> {
        let value: Self = decode(code)?;
        validate(value.magic, value.protocol_version, value.expires_at)?;
        validate_candidates(&value.candidates)?;
        Ok(value)
    }
}

fn validate_candidates(candidates: &[Candidate]) -> Result<()> {
    if candidates.len() > MAX_CANDIDATES {
        return Err(P2pError::InviteInvalid("网络候选数量过多".into()));
    }
    for candidate in candidates {
        candidate.socket_addr()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> Offer {
        Offer {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            session_id: [1; 16],
            host_public_key: vec![1, 2, 3],
            candidates: vec![Candidate {
                address: "127.0.0.1:1".into(),
                kind: CandidateKind::Loopback,
            }],
            expires_at: now().unwrap() + 60,
            nonce: [2; 32],
            capabilities: 1,
        }
    }

    #[test]
    fn offer_round_trip() {
        let value = offer();
        assert_eq!(Offer::decode(&value.encode().unwrap()).unwrap(), value);
    }
    #[test]
    fn candidate_parsing_rejects_invalid_address() {
        let candidate = Candidate {
            address: "not-an-address".into(),
            kind: CandidateKind::Ipv4Direct,
        };
        assert!(matches!(
            candidate.socket_addr(),
            Err(P2pError::InviteInvalid(_))
        ));
    }
    #[test]
    fn invalid_invite_is_rejected() {
        assert!(matches!(
            Offer::decode("bad"),
            Err(P2pError::InviteInvalid(_))
        ));
    }
    #[test]
    fn malformed_packet_is_rejected() {
        assert!(matches!(
            Offer::decode("CCL://bm90LXotc3Rk"),
            Err(P2pError::InviteInvalid(_))
        ));
    }
    #[test]
    fn expired_invite_is_rejected() {
        let mut value = offer();
        value.expires_at = 1;
        assert!(matches!(
            Offer::decode(&value.encode().unwrap()),
            Err(P2pError::InviteExpired)
        ));
    }
    #[test]
    fn version_mismatch_is_rejected() {
        let mut value = offer();
        value.protocol_version += 1;
        assert!(matches!(
            Offer::decode(&value.encode().unwrap()),
            Err(P2pError::ProtocolMismatch { .. })
        ));
    }
}
