use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::Emitter;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    net::TcpStream,
    sync::mpsc,
};

use super::{P2pError, Result};

const MAX_INITIAL_TAIL: u64 = 256 * 1024;
const MAX_LINE_BUFFER: usize = 16 * 1024;
const LAN_MARKERS: [&str; 4] = [
    "Local game hosted on port ",
    "Started serving on ",
    "Started serving on port ",
    "本地游戏已在端口 ",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftLanStatus {
    pub port: u16,
    pub address: String,
    pub ready: bool,
}

pub fn parse_lan_port(text: &str) -> Option<u16> {
    text.lines().rev().find_map(|line| {
        LAN_MARKERS.iter().find_map(|marker| {
            let tail = line
                .split_once(marker)?
                .1
                .trim_start_matches([':', '='])
                .trim_start();
            let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
            let port = digits.parse::<u16>().ok()?;
            (port > 0).then_some(port)
        })
    })
}

async fn verify_port(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_millis(350),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

async fn read_new(path: &Path, offset: &mut u64, pending: &mut String) -> Result<Option<u16>> {
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let length = file.metadata().await?.len();
    if length < *offset {
        *offset = 0;
        pending.clear();
    }
    file.seek(std::io::SeekFrom::Start(*offset)).await?;
    let mut bytes =
        Vec::with_capacity((length.saturating_sub(*offset)).min(MAX_INITIAL_TAIL) as usize);
    file.read_to_end(&mut bytes).await?;
    *offset = length;
    pending.push_str(&String::from_utf8_lossy(&bytes));
    if pending.len() > MAX_LINE_BUFFER {
        let keep_from = pending.len() - MAX_LINE_BUFFER;
        pending.drain(..keep_from);
    }
    let port = parse_lan_port(pending);
    if pending.ends_with('\n') || pending.ends_with('\r') {
        pending.clear();
    } else if let Some(index) = pending.rfind(['\n', '\r']) {
        pending.drain(..=index);
    }
    Ok(port)
}

pub async fn wait_for_lan(log_path: PathBuf, timeout: Duration) -> Result<MinecraftLanStatus> {
    let parent = log_path
        .parent()
        .ok_or_else(|| P2pError::MinecraftLanNotFound)?
        .to_path_buf();
    tokio::fs::create_dir_all(&parent).await?;
    let (events_tx, mut events_rx) = mpsc::channel::<()>(8);
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                let _ = events_tx.try_send(());
            }
        })
        .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))?;

    let initial_length = tokio::fs::metadata(&log_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let mut offset = initial_length.saturating_sub(MAX_INITIAL_TAIL);
    let mut pending = String::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(port) = read_new(&log_path, &mut offset, &mut pending).await? {
            if verify_port(port).await {
                return Ok(MinecraftLanStatus {
                    port,
                    address: format!("127.0.0.1:{port}"),
                    ready: true,
                });
            }
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(P2pError::MinecraftLanNotFound);
        }
        if tokio::time::timeout(remaining.min(Duration::from_secs(2)), events_rx.recv())
            .await
            .is_err()
        {
            continue;
        }
    }
}

#[tauri::command]
pub async fn detect_minecraft_lan(
    app: tauri::AppHandle,
    instance_path: String,
    timeout_secs: Option<u64>,
) -> std::result::Result<MinecraftLanStatus, String> {
    let root = PathBuf::from(instance_path);
    let candidates = [
        root.join("logs/latest.log"),
        root.join(".minecraft/logs/latest.log"),
    ];
    let log = candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| root.join("logs/latest.log"));
    let status = wait_for_lan(
        log,
        Duration::from_secs(timeout_secs.unwrap_or(120).clamp(1, 600)),
    )
    .await
    .map_err(|error| error.to_string())?;
    let _ = app.emit("p2p-minecraft-lan", &status);
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    #[test]
    fn parses_supported_log_formats_and_rejects_bad_ports() {
        assert_eq!(
            parse_lan_port("[Server thread/INFO]: Local game hosted on port 51342"),
            Some(51342)
        );
        assert_eq!(
            parse_lan_port("[Server thread/INFO]: Started serving on 25565"),
            Some(25565)
        );
        assert_eq!(parse_lan_port("Local game hosted on port 99999"), None);
        assert_eq!(parse_lan_port("unrelated 25565"), None);
    }

    #[tokio::test]
    async fn watches_log_and_verifies_live_port() {
        let root = std::env::temp_dir().join(format!("catcl-lan-watch-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();
        let log = root.join("latest.log");
        tokio::fs::write(&log, b"starting\n").await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let watched = tokio::spawn(wait_for_lan(log.clone(), Duration::from_secs(5)));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .await
            .unwrap();
        file.write_all(
            format!("[Server thread/INFO]: Local game hosted on port {port}\n").as_bytes(),
        )
        .await
        .unwrap();
        file.flush().await.unwrap();
        let accepted = tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let status = watched.await.unwrap().unwrap();
        assert_eq!(status.port, port);
        accepted.await.unwrap();
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
