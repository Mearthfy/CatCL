use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;
use tokio::sync::Mutex;

pub const LITTLE_SKIN_API: &str = "https://littleskin.cn/api/yggdrasil";
const MICROSOFT_CLIENT_ID: &str = "eb8e8978-d43c-4810-8d56-9ec6e487bf28";

#[derive(Default)]
pub struct AccountState {
    microsoft: Mutex<Option<PendingMicrosoft>>,
}

#[derive(Clone)]
struct PendingMicrosoft { client_id: String, device_code: String, interval: u64, expires_at: std::time::Instant }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrosoftChallenge { pub user_code: String, pub verification_uri: String, pub expires_in: u64 }

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSession {
    pub mode: String,
    pub player_name: String,
    pub uuid: String,
    pub access_token: String,
    #[serde(default)]
    pub client_token: String,
    #[serde(default)]
    pub xuid: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub mode: String,
    pub player_name: String,
    pub uuid: String,
}

impl From<&AccountSession> for AccountInfo {
    fn from(value: &AccountSession) -> Self {
        Self { mode: value.mode.clone(), player_name: value.player_name.clone(), uuid: value.uuid.clone() }
    }
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|error| error.to_string())?.join("account.json"))
}

async fn save(app: &tauri::AppHandle, account: &AccountSession) -> Result<(), String> {
    let path = path(app)?;
    tokio::fs::create_dir_all(path.parent().unwrap()).await.map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(account).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    tokio::fs::write(&temporary, bytes).await.map_err(|error| error.to_string())?;
    tokio::fs::rename(temporary, path).await.map_err(|error| error.to_string())
}

pub(crate) async fn session(app: &tauri::AppHandle, mode: &str) -> Result<Option<AccountSession>, String> {
    if mode == "offline" { return Ok(None); }
    let bytes = match tokio::fs::read(path(app)?).await {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let account: AccountSession = serde_json::from_slice(&bytes).map_err(|_| "账户会话已损坏，请重新登录".to_owned())?;
    if account.mode != mode { return Ok(None); }
    Ok(Some(account))
}

#[tauri::command]
pub async fn load_account(app: tauri::AppHandle) -> Result<Option<AccountInfo>, String> {
    let bytes = match tokio::fs::read(path(&app)?).await {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let account: AccountSession = serde_json::from_slice(&bytes).map_err(|_| "账户会话已损坏，请重新登录".to_owned())?;
    Ok(Some(AccountInfo::from(&account)))
}

#[tauri::command]
pub async fn logout_account(app: tauri::AppHandle) -> Result<(), String> {
    match tokio::fs::remove_file(path(&app)?).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub async fn login_littleskin(app: tauri::AppHandle, username: String, password: String) -> Result<AccountInfo, String> {
    if username.trim().is_empty() || password.is_empty() { return Err("请输入 LittleSkin 邮箱和密码".into()); }
    let client_token = format!("{:x}", md5::compute(format!("{}-{}", username, std::process::id())));
    let response = reqwest::Client::new()
        .post(format!("{LITTLE_SKIN_API}/authserver/authenticate"))
        .json(&serde_json::json!({"agent":{"name":"Minecraft","version":1},"username":username.trim(),"password":password,"clientToken":client_token,"requestUser":true}))
        .send().await.map_err(|error| format!("无法连接 LittleSkin：{error}"))?;
    let status = response.status();
    let value: serde_json::Value = response.json().await.map_err(|error| format!("LittleSkin 响应无效：{error}"))?;
    if !status.is_success() {
        return Err(value["errorMessage"].as_str().unwrap_or("LittleSkin 登录失败").to_owned());
    }
    let profile = value.get("selectedProfile").filter(|value| !value.is_null())
        .or_else(|| value["availableProfiles"].as_array().and_then(|items| items.first()))
        .ok_or("LittleSkin 账户没有可用的游戏角色")?;
    let account = AccountSession {
        mode: "littleskin".into(),
        player_name: profile["name"].as_str().ok_or("角色缺少名称")?.into(),
        uuid: profile["id"].as_str().ok_or("角色缺少 UUID")?.into(),
        access_token: value["accessToken"].as_str().ok_or("登录响应缺少令牌")?.into(),
        client_token: value["clientToken"].as_str().unwrap_or(&client_token).into(),
        xuid: String::new(),
    };
    save(&app, &account).await?;
    Ok(AccountInfo::from(&account))
}

#[tauri::command]
pub async fn start_microsoft_login(state: tauri::State<'_, AccountState>) -> Result<MicrosoftChallenge, String> {
    let value: serde_json::Value = reqwest::Client::new()
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
        .form(&[("client_id", MICROSOFT_CLIENT_ID), ("scope", "XboxLive.signin offline_access")])
        .send().await.map_err(|error| format!("无法连接 Microsoft：{error}"))?
        .error_for_status().map_err(|error| error.to_string())?.json().await.map_err(|error| error.to_string())?;
    let expires_in = value["expires_in"].as_u64().unwrap_or(900);
    *state.microsoft.lock().await = Some(PendingMicrosoft {
        client_id: MICROSOFT_CLIENT_ID.into(),
        device_code: value["device_code"].as_str().ok_or("Microsoft 未返回设备代码")?.into(),
        interval: value["interval"].as_u64().unwrap_or(5),
        expires_at: std::time::Instant::now() + std::time::Duration::from_secs(expires_in),
    });
    Ok(MicrosoftChallenge {
        user_code: value["user_code"].as_str().ok_or("Microsoft 未返回验证码")?.into(),
        verification_uri: value["verification_uri"].as_str().ok_or("Microsoft 未返回登录网址")?.into(),
        expires_in,
    })
}

async fn post_json(client: &reqwest::Client, url: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let response = client.post(url).json(&body).send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let value: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let message = value["errorMessage"].as_str().or_else(|| value["Message"].as_str()).unwrap_or("Microsoft 认证失败");
        if message.contains("Invalid app registration") {
            return Err(format!(
                "CCL 的 Microsoft Client ID（{MICROSOFT_CLIENT_ID}）尚未加入 Minecraft Services 白名单。Microsoft、Xbox Live 与 XSTS 授权已经成功，但正版登录必须等待 Minecraft 审核该应用注册；修改 Entra 权限无法绕过此限制。"
            ));
        }
        let xerr = value["XErr"].as_u64().map(|code| format!("，Xbox 错误码 {code}" )).unwrap_or_default();
        return Err(format!("{message}{xerr}"));
    }
    Ok(value)
}

#[tauri::command]
pub fn open_microsoft_login(url: String) -> Result<(), String> {
    if !matches!(url.as_str(), "https://microsoft.com/devicelogin" | "https://www.microsoft.com/link") {
        return Err("拒绝打开非 Microsoft 设备登录地址".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn().map_err(|error| format!("无法打开浏览器：{error}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    { Err("当前平台暂不支持自动打开浏览器".into()) }
}

#[tauri::command]
pub async fn finish_microsoft_login(app: tauri::AppHandle, state: tauri::State<'_, AccountState>) -> Result<AccountInfo, String> {
    // Keep the pending session until authentication succeeds so a transient
    // network failure can be retried without asking the user for a new code.
    let pending = state.microsoft.lock().await.as_ref().cloned().ok_or("请先获取 Microsoft 登录验证码")?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30))
        .build().map_err(|error| error.to_string())?;
    let mut transport_failures = 0u8;
    let microsoft_token = loop {
        if std::time::Instant::now() >= pending.expires_at { return Err("Microsoft 登录验证码已过期".into()); }
        tokio::time::sleep(std::time::Duration::from_secs(pending.interval)).await;
        let response = match client.post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", pending.client_id.as_str()), ("device_code", pending.device_code.as_str())])
            .send().await {
                Ok(response) => { transport_failures = 0; response }
                Err(error) => {
                    transport_failures += 1;
                    if transport_failures < 5 { continue; }
                    return Err(format!("连续 5 次无法连接 Microsoft 登录服务，请检查系统代理、防火墙或 TLS：{error}"));
                }
            };
        let value: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
        if let Some(token) = value["access_token"].as_str() { break token.to_owned(); }
        match value["error"].as_str().unwrap_or("") {
            "authorization_pending" => continue,
            "slow_down" => { tokio::time::sleep(std::time::Duration::from_secs(5)).await; continue; }
            _ => return Err(value["error_description"].as_str().unwrap_or("Microsoft 登录未完成").into()),
        }
    };
    let xbox = post_json(&client, "https://user.auth.xboxlive.com/user/authenticate", serde_json::json!({
        "Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={microsoft_token}")},
        "RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"
    })).await?;
    let xbox_token = xbox["Token"].as_str().ok_or("Xbox Live 未返回令牌")?;
    let xsts = post_json(&client, "https://xsts.auth.xboxlive.com/xsts/authorize", serde_json::json!({
        "Properties":{"SandboxId":"RETAIL","UserTokens":[xbox_token]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"
    })).await?;
    let xsts_token = xsts["Token"].as_str().ok_or("XSTS 未返回令牌")?;
    let uhs = xsts["DisplayClaims"]["xui"][0]["uhs"].as_str().ok_or("XSTS 未返回用户标识")?;
    let minecraft = post_json(&client, "https://api.minecraftservices.com/authentication/login_with_xbox", serde_json::json!({
        "identityToken":format!("XBL3.0 x={uhs};{xsts_token}")
    })).await?;
    let access_token = minecraft["access_token"].as_str().ok_or("Minecraft 未返回令牌")?.to_owned();
    let profile: serde_json::Value = client.get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&access_token).send().await.map_err(|error| error.to_string())?
        .error_for_status().map_err(|_| "该 Microsoft 账户没有 Minecraft Java 版档案".to_owned())?
        .json().await.map_err(|error| error.to_string())?;
    let account = AccountSession { mode:"microsoft".into(), player_name:profile["name"].as_str().ok_or("正版档案缺少名称")?.into(), uuid:profile["id"].as_str().ok_or("正版档案缺少 UUID")?.into(), access_token, client_token:String::new(), xuid:uhs.into() };
    save(&app, &account).await?;
    *state.microsoft.lock().await = None;
    Ok(AccountInfo::from(&account))
}

pub(crate) async fn ensure_authlib_injector(app: &tauri::AppHandle) -> Result<(PathBuf, String), String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let directory = app.path().app_cache_dir().map_err(|error| error.to_string())?.join("authlib-injector");
    let jar = directory.join("authlib-injector.jar");
    tokio::fs::create_dir_all(&directory).await.map_err(|error| error.to_string())?;
    if tokio::fs::metadata(&jar).await.is_err() {
        let metadata: serde_json::Value = reqwest::get("https://authlib-injector.yushi.moe/artifact/latest.json")
            .await.map_err(|error| format!("无法读取 authlib-injector 版本：{error}"))?
            .error_for_status().map_err(|error| error.to_string())?.json().await.map_err(|error| error.to_string())?;
        let url = metadata["download_url"].as_str().ok_or("authlib-injector 下载信息缺少 download_url")?;
        let bytes = reqwest::get(url).await.map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?.bytes().await.map_err(|error| error.to_string())?;
        tokio::fs::write(&jar, bytes).await.map_err(|error| error.to_string())?;
    }
    let metadata = reqwest::get(LITTLE_SKIN_API).await.map_err(|error| error.to_string())?.error_for_status().map_err(|error| error.to_string())?.bytes().await.map_err(|error| error.to_string())?;
    Ok((jar, STANDARD.encode(metadata)))
}
