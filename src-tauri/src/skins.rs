use crate::settings::validate_offline_name;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Manager;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

const MAX_SKIN_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinInfo {
    player: String,
    width: u32,
    height: u32,
    data_url: String,
}

fn skin_path(app: &tauri::AppHandle, player: &str) -> Result<PathBuf, String> {
    validate_offline_name(player)?;
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("skins")
        .join(format!("{}.png", player.to_ascii_lowercase())))
}

pub(crate) async fn skin_bytes(app: &tauri::AppHandle, player: &str) -> Option<Vec<u8>> {
    tokio::fs::read(skin_path(app, player).ok()?).await.ok()
}

/// Makes an imported launcher skin visible to CustomSkinLoader when an
/// instance already contains that mod. LocalSkin must precede Mojang because
/// an offline name can also belong to a real account (for example `Player`).
pub(crate) async fn stage_for_instance(
    game_directory: &Path,
    player: &str,
    bytes: &[u8],
) -> Result<bool, String> {
    validate_offline_name(player)?;
    let data_directory = game_directory.join("CustomSkinLoader");
    let config_path = data_directory.join("CustomSkinLoader.json");
    if tokio::fs::metadata(&config_path).await.is_err() {
        return Ok(false);
    }

    let skin_directory = data_directory.join("LocalSkin").join("skins");
    tokio::fs::create_dir_all(&skin_directory)
        .await
        .map_err(|error| error.to_string())?;
    let destination = skin_directory.join(format!("{player}.png"));
    let temporary = skin_directory.join(format!(".{player}.catcl.tmp"));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::rename(&temporary, &destination)
        .await
        .map_err(|error| error.to_string())?;

    let config_bytes = tokio::fs::read(&config_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut config: serde_json::Value =
        serde_json::from_slice(&config_bytes).map_err(|error| error.to_string())?;
    if let Some(loaders) = config.get_mut("loadlist").and_then(|value| value.as_array_mut()) {
        if let Some(index) = loaders.iter().position(|loader| {
            loader.get("name").and_then(|value| value.as_str()) == Some("LocalSkin")
        }) {
            let local = loaders.remove(index);
            loaders.insert(0, local);
            let serialized = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
            let temporary = config_path.with_extension("json.catcl.tmp");
            tokio::fs::write(&temporary, serialized)
                .await
                .map_err(|error| error.to_string())?;
            tokio::fs::rename(temporary, config_path)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(true)
}

pub(crate) struct SkinServer {
    pub url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for SkinServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

pub(crate) async fn serve(bytes: Vec<u8>) -> Result<SkinServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let token = format!("{:x}", md5::compute(&bytes));
    let route = format!("/{token}.png");
    let url = format!("http://{address}{route}");
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = &mut shutdown_rx => break,
                value = listener.accept() => value,
            };
            let Ok((mut socket, _)) = accepted else { break };
            let skin = bytes.clone();
            let expected = route.clone();
            tokio::spawn(async move {
                let mut request = [0u8; 4096];
                let count = socket.read(&mut request).await.unwrap_or(0);
                let first = String::from_utf8_lossy(&request[..count]);
                let valid = first
                    .lines()
                    .next()
                    .is_some_and(|line| line.starts_with(&format!("GET {expected} ")));
                if valid {
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", skin.len());
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&skin).await;
                } else {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                }
            });
        }
    });
    Ok(SkinServer {
        url,
        shutdown: Some(shutdown_tx),
    })
}

fn inspect_png(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("请选择有效的 PNG 皮肤文件".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width != 64 || !matches!(height, 32 | 64) {
        return Err("皮肤尺寸必须为 64×64，或兼容旧版的 64×32".into());
    }
    Ok((width, height))
}

fn info(player: &str, bytes: Vec<u8>) -> Result<SkinInfo, String> {
    let (width, height) = inspect_png(&bytes)?;
    Ok(SkinInfo {
        player: player.into(),
        width,
        height,
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
    })
}

#[tauri::command]
pub async fn import_skin(
    app: tauri::AppHandle,
    player: String,
    path: String,
) -> Result<SkinInfo, String> {
    validate_offline_name(&player)?;
    let source = Path::new(&path);
    if !source.is_absolute()
        || !source
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("png"))
    {
        return Err("请选择电脑上的 PNG 皮肤文件".into());
    }
    let bytes = tokio::fs::read(source)
        .await
        .map_err(|e| format!("无法读取皮肤：{e}"))?;
    if bytes.len() > MAX_SKIN_BYTES {
        return Err("皮肤文件不能超过 2MB".into());
    }
    let result = info(&player, bytes.clone())?;
    let destination = skin_path(&app, &player)?;
    tokio::fs::create_dir_all(destination.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    let temp = destination.with_extension("png.tmp");
    tokio::fs::write(&temp, bytes)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(temp, destination)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result)
}

#[tauri::command]
pub async fn load_skin(app: tauri::AppHandle, player: String) -> Result<Option<SkinInfo>, String> {
    match tokio::fs::read(skin_path(&app, &player)?).await {
        Ok(bytes) => info(&player, bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub async fn remove_skin(app: tauri::AppHandle, player: String) -> Result<(), String> {
    match tokio::fs::remove_file(skin_path(&app, &player)?).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_skin_dimensions() {
        let mut png = vec![0; 24];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[12..16].copy_from_slice(b"IHDR");
        png[16..20].copy_from_slice(&64u32.to_be_bytes());
        png[20..24].copy_from_slice(&64u32.to_be_bytes());
        assert_eq!(inspect_png(&png).unwrap(), (64, 64));
        png[16..20].copy_from_slice(&128u32.to_be_bytes());
        assert!(inspect_png(&png).is_err());
    }

    #[tokio::test]
    async fn stages_skin_and_prioritizes_local_custom_skin_loader() {
        let root = std::env::temp_dir().join(format!("catcl-skin-test-{}", std::process::id()));
        let data = root.join("CustomSkinLoader");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&data).await.unwrap();
        tokio::fs::write(
            data.join("CustomSkinLoader.json"),
            br#"{"loadlist":[{"name":"Mojang"},{"name":"LocalSkin","type":"Legacy"}]}"#,
        )
        .await
        .unwrap();
        assert!(stage_for_instance(&root, "Player", b"png").await.unwrap());
        assert_eq!(
            tokio::fs::read(data.join("LocalSkin/skins/Player.png"))
                .await
                .unwrap(),
            b"png"
        );
        let config: serde_json::Value = serde_json::from_slice(
            &tokio::fs::read(data.join("CustomSkinLoader.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(config["loadlist"][0]["name"], "LocalSkin");
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
