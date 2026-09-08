use super::{
    invite::{Answer, Candidate, Offer},
    protocol::{MAGIC, PROTOCOL_VERSION},
    P2pError, Result,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_MAX_PLAYERS: usize = 4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    CreatingSession,
    DetectingNetwork,
    WaitingForAnswer,
    Connecting,
    HolePunching,
    Handshaking,
    Connected,
    MinecraftReady,
    Closing,
    Closed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizedPeer {
    pub peer_id: [u8; 16],
    pub nonce: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: [u8; 16],
    pub state: SessionState,
    pub host_nonce: [u8; 32],
    pub peers: HashMap<[u8; 16], AuthorizedPeer>,
    pub max_players: usize,
}

impl Session {
    pub fn new(max_players: usize) -> Self {
        let mut id = [0; 16];
        let mut nonce = [0; 32];
        OsRng.fill_bytes(&mut id);
        OsRng.fill_bytes(&mut nonce);
        Self {
            id,
            state: SessionState::Idle,
            host_nonce: nonce,
            peers: HashMap::new(),
            max_players: max_players.clamp(2, DEFAULT_MAX_PLAYERS),
        }
    }
    pub fn transition(&mut self, next: SessionState) -> Result<()> {
        use SessionState::*;
        let valid = matches!(
            (self.state, next),
            (Idle, CreatingSession)
                | (CreatingSession, DetectingNetwork)
                | (DetectingNetwork, WaitingForAnswer)
                | (WaitingForAnswer, Connecting)
                | (Connecting, Handshaking)
                | (Handshaking, Connected)
                | (Connected, MinecraftReady)
                | (_, Closing)
                | (Closing, Closed)
                | (_, Failed)
        );
        if !valid {
            return Err(P2pError::InvalidState(format!(
                "{:?} -> {:?}",
                self.state, next
            )));
        }
        self.state = next;
        Ok(())
    }
    pub fn offer(
        &self,
        certificate: Vec<u8>,
        candidates: Vec<Candidate>,
        ttl: Duration,
    ) -> Result<Offer> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| P2pError::InviteInvalid(e.to_string()))?
            .as_secs();
        Ok(Offer {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            session_id: self.id,
            host_public_key: certificate,
            candidates,
            expires_at: now + ttl.as_secs(),
            nonce: self.host_nonce,
            capabilities: 1,
        })
    }
    pub fn authorize(&mut self, answer: Answer) -> Result<()> {
        if answer.session_id != self.id || answer.offer_nonce != self.host_nonce {
            return Err(P2pError::HandshakeFailed("响应与当前会话不匹配".into()));
        }
        if !self.peers.contains_key(&answer.peer_id) && self.peers.len() + 1 >= self.max_players {
            return Err(P2pError::PlayerLimitReached(self.max_players));
        }
        self.peers.insert(
            answer.peer_id,
            AuthorizedPeer {
                peer_id: answer.peer_id,
                nonce: answer.peer_nonce,
            },
        );
        Ok(())
    }
    pub fn verify(&self, peer_id: &[u8; 16], nonce: &[u8; 32]) -> bool {
        self.peers
            .get(peer_id)
            .is_some_and(|peer| &peer.nonce == nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transitions_are_checked() {
        let mut session = Session::new(4);
        assert!(session.transition(SessionState::Connected).is_err());
        session.transition(SessionState::CreatingSession).unwrap();
        assert_eq!(session.state, SessionState::CreatingSession);
    }
    #[test]
    fn session_serialization_round_trip() {
        let session = Session::new(4);
        let bytes = bincode::serialize(&session).unwrap();
        let restored: Session = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.id, session.id);
        assert_eq!(restored.state, session.state);
    }
    #[test]
    fn player_limit_is_enforced() {
        let mut session = Session::new(2);
        let first = Answer {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            session_id: session.id,
            peer_id: [4; 16],
            offer_nonce: session.host_nonce,
            peer_nonce: [5; 32],
            candidates: vec![],
            expires_at: u64::MAX,
        };
        session.authorize(first).unwrap();
        let answer = Answer {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            session_id: session.id,
            peer_id: [6; 16],
            offer_nonce: session.host_nonce,
            peer_nonce: [7; 32],
            candidates: vec![],
            expires_at: u64::MAX,
        };
        assert!(matches!(
            session.authorize(answer),
            Err(P2pError::PlayerLimitReached(2))
        ));
    }
}
