use thiserror::Error;

pub type Result<T> = std::result::Result<T, P2pError>;

#[derive(Debug, Error)]
pub enum P2pError {
    #[error("网络不可用：{0}")]
    NetworkUnavailable(String),
    #[error("连接超时")]
    Timeout,
    #[error("邀请代码无效：{0}")]
    InviteInvalid(String),
    #[error("邀请代码已过期")]
    InviteExpired,
    #[error("协议版本不兼容：需要 {expected}，收到 {actual}")]
    ProtocolMismatch { expected: u16, actual: u16 },
    #[error("加密初始化失败：{0}")]
    CryptoError(String),
    #[error("握手失败：{0}")]
    HandshakeFailed(String),
    #[error("已达到最大玩家数 {0}")]
    PlayerLimitReached(usize),
    #[error("会话状态不允许此操作：{0}")]
    InvalidState(String),
    #[error("代理失败：{0}")]
    Proxy(String),
    #[error("对端已断开")]
    PeerDisconnected,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Encode(#[from] Box<bincode::ErrorKind>),
}
