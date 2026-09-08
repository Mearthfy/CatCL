use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use tauri::Emitter;
use tokio::sync::{Mutex, RwLock};

use super::{
    candidate::preferred_direct,
    invite::{Answer, CandidateKind, Offer},
    minecraft::{wait_for_lan, MinecraftLanStatus},
    network::inspect,
    protocol::{StreamHandshake, MAGIC, PROTOCOL_VERSION},
    proxy::LocalProxy,
    session::{Session, SessionState, DEFAULT_MAX_PLAYERS},
    transport::{connect, HostTransport},
    P2pError, Result,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshot {
    pub role: String,
    pub state: SessionState,
    pub invite_code: Option<String>,
    pub answer_code: Option<String>,
    pub minecraft_lan_port: Option<u16>,
    pub local_address: Option<String>,
    pub player_count: usize,
    pub transport: Option<String>,
    pub message: String,
}

enum RoomRuntime {
    Host {
        session: Arc<RwLock<Session>>,
        transport: HostTransport,
        offer: Offer,
        lan: MinecraftLanStatus,
    },
    ClientPending {
        offer: Offer,
        answer: Answer,
    },
    ClientConnected {
        endpoint: quinn::Endpoint,
        proxy: LocalProxy,
        offer: Offer,
        answer: Answer,
    },
}

#[derive(Default)]
pub struct RoomState(Mutex<Option<RoomRuntime>>);

fn emit(app: &tauri::AppHandle, snapshot: &RoomSnapshot) {
    let _ = app.emit("p2p-room-state", snapshot);
}

fn empty_snapshot(state: SessionState, message: impl Into<String>) -> RoomSnapshot {
    RoomSnapshot {
        role: "none".into(),
        state,
        invite_code: None,
        answer_code: None,
        minecraft_lan_port: None,
        local_address: None,
        player_count: 0,
        transport: None,
        message: message.into(),
    }
}

fn host_snapshot(
    session: &Session,
    offer: &Offer,
    lan: &MinecraftLanStatus,
    message: impl Into<String>,
) -> Result<RoomSnapshot> {
    Ok(RoomSnapshot {
        role: "host".into(),
        state: session.state,
        invite_code: Some(offer.encode()?),
        answer_code: None,
        minecraft_lan_port: Some(lan.port),
        local_address: None,
        player_count: session.peers.len() + 1,
        transport: Some(
            if offer
                .candidates
                .iter()
                .any(|item| matches!(item.kind, CandidateKind::Ipv6Direct))
            {
                "IPv6 Direct"
            } else {
                "P2P Direct"
            }
            .into(),
        ),
        message: message.into(),
    })
}

fn client_snapshot(
    offer: &Offer,
    answer: &Answer,
    state: SessionState,
    local: Option<String>,
    message: impl Into<String>,
) -> Result<RoomSnapshot> {
    Ok(RoomSnapshot {
        role: "client".into(),
        state,
        invite_code: None,
        answer_code: Some(answer.encode()?),
        minecraft_lan_port: None,
        local_address: local,
        player_count: 1,
        transport: Some(
            if offer
                .candidates
                .iter()
                .any(|item| matches!(item.kind, CandidateKind::Ipv6Direct))
            {
                "IPv6 Direct"
            } else {
                "P2P Direct"
            }
            .into(),
        ),
        message: message.into(),
    })
}

fn log_path(instance_path: &str) -> PathBuf {
    let root = PathBuf::from(instance_path);
    let direct = root.join("logs/latest.log");
    if direct.exists() {
        direct
    } else {
        root.join(".minecraft/logs/latest.log")
    }
}

#[tauri::command]
pub async fn create_p2p_room(
    app: tauri::AppHandle,
    state: tauri::State<'_, RoomState>,
    instance_path: String,
    lan_timeout_secs: Option<u64>,
) -> std::result::Result<RoomSnapshot, String> {
    {
        let active = state.0.lock().await;
        if active.is_some() {
            return Err("已有活动的联机房间，请先关闭".into());
        }
    }
    emit(
        &app,
        &empty_snapshot(
            SessionState::DetectingNetwork,
            "正在等待 Minecraft 对局域网开放…",
        ),
    );
    let lan = wait_for_lan(
        log_path(&instance_path),
        Duration::from_secs(lan_timeout_secs.unwrap_or(120).clamp(1, 600)),
    )
    .await
    .map_err(|error| error.to_string())?;
    let session = Arc::new(RwLock::new(Session::new(DEFAULT_MAX_PLAYERS)));
    {
        let mut value = session.write().await;
        value
            .transition(SessionState::CreatingSession)
            .map_err(|error| error.to_string())?;
        value
            .transition(SessionState::DetectingNetwork)
            .map_err(|error| error.to_string())?;
    }
    let network = inspect(0).await.map_err(|error| error.to_string())?;
    let use_ipv6 = network.ipv6_direct;
    let bind: SocketAddr = if use_ipv6 { "[::]:0" } else { "0.0.0.0:0" }
        .parse()
        .map_err(|_| "内部监听地址无效")?;
    let target = SocketAddr::from(([127, 0, 0, 1], lan.port));
    let transport = HostTransport::start(bind, target, session.clone())
        .await
        .map_err(|error| error.to_string())?;
    let mut network = inspect(transport.address.port())
        .await
        .map_err(|error| error.to_string())?;
    network.candidates.retain(|candidate| {
        candidate
            .socket_addr()
            .is_ok_and(|address| address.is_ipv6() == use_ipv6)
    });
    if network.candidates.is_empty() {
        transport.shutdown().await;
        return Err("没有可用于建立房间的网络地址".into());
    }
    let offer = {
        let mut value = session.write().await;
        value
            .transition(SessionState::WaitingForAnswer)
            .map_err(|error| error.to_string())?;
        value
            .offer(
                transport.certificate.clone(),
                network.candidates,
                Duration::from_secs(1800),
            )
            .map_err(|error| error.to_string())?
    };
    let snapshot = {
        let value = session.read().await;
        host_snapshot(&value, &offer, &lan, "房间已创建，等待好友回应码")
            .map_err(|error| error.to_string())?
    };
    *state.0.lock().await = Some(RoomRuntime::Host {
        session,
        transport,
        offer,
        lan,
    });
    emit(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn join_p2p_room(
    app: tauri::AppHandle,
    state: tauri::State<'_, RoomState>,
    invite_code: String,
) -> std::result::Result<RoomSnapshot, String> {
    let offer = Offer::decode(&invite_code).map_err(|error| error.to_string())?;
    let mut peer_id = [0; 16];
    let mut peer_nonce = [0; 32];
    OsRng.fill_bytes(&mut peer_id);
    OsRng.fill_bytes(&mut peer_nonce);
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs()
        + 1800;
    let answer = Answer {
        magic: MAGIC,
        protocol_version: PROTOCOL_VERSION,
        session_id: offer.session_id,
        peer_id,
        offer_nonce: offer.nonce,
        peer_nonce,
        candidates: vec![],
        expires_at: expires_at.min(offer.expires_at),
    };
    let snapshot = client_snapshot(
        &offer,
        &answer,
        SessionState::WaitingForAnswer,
        None,
        "回应码已生成，请发送给房主",
    )
    .map_err(|error| error.to_string())?;
    let mut active = state.0.lock().await;
    if active.is_some() {
        return Err("已有活动的联机房间，请先关闭".into());
    }
    *active = Some(RoomRuntime::ClientPending { offer, answer });
    emit(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn accept_p2p_answer(
    app: tauri::AppHandle,
    state: tauri::State<'_, RoomState>,
    answer_code: String,
) -> std::result::Result<RoomSnapshot, String> {
    let answer = Answer::decode(&answer_code).map_err(|error| error.to_string())?;
    let active = state.0.lock().await;
    let RoomRuntime::Host {
        session,
        offer,
        lan,
        ..
    } = active.as_ref().ok_or("没有活动的房主房间")?
    else {
        return Err("当前不是房主房间".into());
    };
    let mut value = session.write().await;
    value.authorize(answer).map_err(|error| error.to_string())?;
    let snapshot = host_snapshot(&value, offer, lan, "好友已授权，可以通知对方建立连接")
        .map_err(|error| error.to_string())?;
    emit(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn connect_p2p_room(
    app: tauri::AppHandle,
    state: tauri::State<'_, RoomState>,
) -> std::result::Result<RoomSnapshot, String> {
    let pending = {
        let mut active = state.0.lock().await;
        match active.take() {
            Some(RoomRuntime::ClientPending { offer, answer }) => (offer, answer),
            Some(other) => {
                *active = Some(other);
                return Err("当前没有等待连接的加入请求".into());
            }
            None => return Err("请先导入房主邀请码".into()),
        }
    };
    let (offer, answer) = pending;
    let target = preferred_direct(&offer.candidates)
        .ok_or_else(|| P2pError::NetworkUnavailable("邀请码没有可用直连地址".into()).to_string())?;
    let (endpoint, connection) = match connect(target, offer.host_public_key.clone()).await {
        Ok(value) => value,
        Err(error) => {
            *state.0.lock().await = Some(RoomRuntime::ClientPending { offer, answer });
            return Err(error.to_string());
        }
    };
    let handshake = StreamHandshake::new(offer.session_id, answer.peer_id, answer.peer_nonce);
    let proxy = match LocalProxy::start(
        "127.0.0.1:25565".parse().expect("static address"),
        connection.clone(),
        handshake.clone(),
    )
    .await
    {
        Ok(value) => value,
        Err(error)
            if error.to_string().contains("address") || error.to_string().contains("地址") =>
        {
            LocalProxy::start(
                "127.0.0.1:0".parse().expect("static address"),
                connection,
                handshake,
            )
            .await
            .map_err(|error| error.to_string())?
        }
        Err(error) => return Err(error.to_string()),
    };
    let local = proxy.address.to_string();
    let snapshot = client_snapshot(
        &offer,
        &answer,
        SessionState::Connected,
        Some(local.clone()),
        "联机已建立，在多人游戏中连接此地址",
    )
    .map_err(|error| error.to_string())?;
    *state.0.lock().await = Some(RoomRuntime::ClientConnected {
        endpoint,
        proxy,
        offer,
        answer,
    });
    emit(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn p2p_room_status(
    state: tauri::State<'_, RoomState>,
) -> std::result::Result<RoomSnapshot, String> {
    let active = state.0.lock().await;
    match active.as_ref() {
        None => Ok(empty_snapshot(SessionState::Idle, "尚未创建或加入房间")),
        Some(RoomRuntime::Host {
            session,
            offer,
            lan,
            ..
        }) => {
            let session = session.read().await;
            host_snapshot(&session, offer, lan, "等待或管理好友连接")
                .map_err(|error| error.to_string())
        }
        Some(RoomRuntime::ClientPending { offer, answer }) => client_snapshot(
            offer,
            answer,
            SessionState::WaitingForAnswer,
            None,
            "等待房主导入回应码",
        )
        .map_err(|error| error.to_string()),
        Some(RoomRuntime::ClientConnected {
            proxy,
            offer,
            answer,
            ..
        }) => client_snapshot(
            offer,
            answer,
            SessionState::Connected,
            Some(proxy.address.to_string()),
            "联机已建立",
        )
        .map_err(|error| error.to_string()),
    }
}

#[tauri::command]
pub async fn close_p2p_room(
    app: tauri::AppHandle,
    state: tauri::State<'_, RoomState>,
) -> std::result::Result<RoomSnapshot, String> {
    let active = state.0.lock().await.take();
    match active {
        Some(RoomRuntime::Host { transport, .. }) => transport.shutdown().await,
        Some(RoomRuntime::ClientConnected {
            endpoint, proxy, ..
        }) => {
            proxy.shutdown().await;
            endpoint.close(0u32.into(), b"room closed");
        }
        _ => {}
    }
    let snapshot = empty_snapshot(SessionState::Closed, "联机房间已关闭");
    emit(&app, &snapshot);
    Ok(snapshot)
}
