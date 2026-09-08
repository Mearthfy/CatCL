use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, PathBuf},
    process::Command,
    time::Duration,
};
use tauri::Emitter;
use tokio::io::AsyncWriteExt;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceProject {
    #[serde(rename(deserialize = "project_id", serialize = "id"))]
    id: String,
    title: String,
    description: String,
    #[serde(rename(deserialize = "project_type", serialize = "projectType"))]
    project_type: String,
    #[serde(rename(deserialize = "icon_url", serialize = "iconUrl"))]
    icon_url: Option<String>,
    downloads: u64,
    author: String,
}

#[derive(Deserialize)]
struct SearchResponse {
    hits: Vec<ResourceProject>,
}

#[derive(Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    primary: bool,
    hashes: std::collections::HashMap<String, String>,
}
#[derive(Deserialize)]
struct ProjectVersion {
    project_id: String,
    files: Vec<VersionFile>,
}

#[derive(Deserialize)]
struct ApiDependency {
    project_id: Option<String>,
    version_id: Option<String>,
    dependency_type: String,
}
#[derive(Deserialize)]
struct ApiVersion {
    id: String,
    name: String,
    version_number: String,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    date_published: String,
    dependencies: Vec<ApiDependency>,
}
#[derive(Deserialize)]
struct ProjectTitle {
    id: String,
    title: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDependency {
    project_id: Option<String>,
    version_id: Option<String>,
    dependency_type: String,
    title: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceVersionInfo {
    id: String,
    name: String,
    version_number: String,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    date_published: String,
    dependencies: Vec<ResourceDependency>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("CatCL/0.1.0 (Minecraft launcher)")
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())
}

fn chinese_alias(query: &str) -> Option<&'static str> {
    [
        ("暮色森林", "twilight forest"),
        ("旅行地图", "journeymap"),
        ("小地图", "minimap"),
        ("物品管理器", "just enough items"),
        ("钠", "sodium"),
        ("锂", "lithium"),
        ("磷", "phosphor"),
        ("补充光影", "complementary shaders"),
        ("光影", "shader"),
        ("空岛", "skyblock"),
        ("科技", "technology"),
        ("魔法", "magic"),
        ("冒险", "adventure"),
        ("优化", "optimization"),
        ("资源包", "resource pack"),
    ]
    .into_iter()
    .find(|(chinese, _)| query.contains(chinese))
    .map(|(_, english)| english)
}

fn encode_query(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (*byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[tauri::command]
pub fn open_resource_site(source: String, query: String) -> Result<(), String> {
    let url = match source.as_str() {
        "mcmod" => format!("https://www.mcmod.cn/s?key={}", encode_query(query.trim())),
        "curseforge" => format!(
            "https://www.curseforge.com/minecraft/search?page=1&pageSize=20&sortBy=relevancy&class=mc-mods&search={}",
            encode_query(query.trim())
        ),
        _ => return Err("不支持的资源站点".into()),
    };
    Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map_err(|e| format!("无法打开系统浏览器：{e}"))?;
    Ok(())
}

async fn run_search(
    http: &reqwest::Client,
    query: &str,
    facets: &str,
) -> Result<Vec<ResourceProject>, String> {
    Ok(http
        .get("https://api.modrinth.com/v2/search")
        .query(&[
            ("query", query),
            ("limit", "30"),
            ("index", "downloads"),
            ("facets", facets),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<SearchResponse>()
        .await
        .map_err(|e| format!("无法解析 Modrinth 搜索结果：{e}"))?
        .hits)
}

#[tauri::command]
pub async fn search_resources(
    query: String,
    project_type: String,
    game_version: String,
    loader: String,
) -> Result<Vec<ResourceProject>, String> {
    let allowed = ["mod", "modpack", "shader", "resourcepack", "datapack"];
    if !allowed.contains(&project_type.as_str()) {
        return Err("不支持的资源类型".into());
    }
    let mut facets = vec![vec![format!("project_type:{project_type}")]];
    if !game_version.is_empty() {
        facets.push(vec![format!("versions:{game_version}")]);
    }
    if project_type == "mod" && !loader.is_empty() {
        facets.push(vec![format!("categories:{loader}")]);
    }
    let http = client()?;
    let facets = serde_json::to_string(&facets).map_err(|e| e.to_string())?;
    let mut hits = if let Some(alias) = chinese_alias(&query) {
        let mut translated = run_search(&http, alias, &facets).await?;
        let original = run_search(&http, &query, &facets).await?;
        let mut seen: std::collections::HashSet<String> = translated
            .iter()
            .map(|project| project.id.clone())
            .collect();
        translated.extend(
            original
                .into_iter()
                .filter(|project| seen.insert(project.id.clone())),
        );
        translated
    } else {
        run_search(&http, &query, &facets).await?
    };
    if hits.len() > 30 {
        hits.truncate(30);
    }
    Ok(hits)
}

#[tauri::command]
pub async fn resource_versions(
    project_id: String,
    game_version: String,
    loader: String,
) -> Result<Vec<ResourceVersionInfo>, String> {
    let http = client()?;
    let mut request = http.get(format!(
        "https://api.modrinth.com/v2/project/{project_id}/version"
    ));
    if !game_version.is_empty() {
        request = request.query(&[(
            "game_versions",
            serde_json::to_string(&vec![&game_version]).unwrap(),
        )]);
    }
    if !loader.is_empty() {
        request = request.query(&[("loaders", serde_json::to_string(&vec![&loader]).unwrap())]);
    }
    let versions: Vec<ApiVersion> = request
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let dependency_ids: std::collections::HashSet<String> = versions
        .iter()
        .flat_map(|version| version.dependencies.iter())
        .filter_map(|dependency| dependency.project_id.clone())
        .collect();
    let dependency_titles: std::collections::HashMap<String, String> = if dependency_ids.is_empty()
    {
        std::collections::HashMap::new()
    } else {
        let ids = serde_json::to_string(&dependency_ids).map_err(|e| e.to_string())?;
        http.get("https://api.modrinth.com/v2/projects")
            .query(&[("ids", ids)])
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<Vec<ProjectTitle>>()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|project| (project.id, project.title))
            .collect()
    };
    let mut result = Vec::with_capacity(versions.len());
    for version in versions {
        let mut dependencies = Vec::new();
        for dependency in version.dependencies {
            let title = if let Some(id) = &dependency.project_id {
                dependency_titles
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.clone())
            } else {
                dependency
                    .version_id
                    .clone()
                    .unwrap_or_else(|| "未知依赖".into())
            };
            dependencies.push(ResourceDependency {
                project_id: dependency.project_id,
                version_id: dependency.version_id,
                dependency_type: dependency.dependency_type,
                title,
            });
        }
        result.push(ResourceVersionInfo {
            id: version.id,
            name: version.name,
            version_number: version.version_number,
            game_versions: version.game_versions,
            loaders: version.loaders,
            date_published: version.date_published,
            dependencies,
        });
    }
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInstallResult {
    path: String,
    project_type: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceProgress {
    id: String,
    downloaded: u64,
    total: u64,
    message: String,
}

fn emit_resource_progress(
    app: &tauri::AppHandle,
    id: &str,
    downloaded: u64,
    total: u64,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "resource-progress",
        ResourceProgress {
            id: id.into(),
            downloaded,
            total,
            message: message.into(),
        },
    );
}

async fn download_to_file(
    app: &tauri::AppHandle,
    http: &reqwest::Client,
    id: &str,
    url: &str,
    target: &std::path::Path,
    expected_sha1: Option<&String>,
) -> Result<(), String> {
    use sha1::Digest;
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let total = response.content_length().unwrap_or(0);
    let temp = target.with_extension("catcl.part");
    let mut output = tokio::fs::File::create(&temp)
        .await
        .map_err(|e| e.to_string())?;
    let mut digest = sha1::Sha1::new();
    let mut downloaded = 0u64;
    let mut chunks = response.bytes_stream();
    emit_resource_progress(app, id, 0, total, "正在下载资源文件…");
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        output.write_all(&chunk).await.map_err(|e| e.to_string())?;
        digest.update(&chunk);
        downloaded += chunk.len() as u64;
        emit_resource_progress(app, id, downloaded, total, "正在下载资源文件…");
    }
    output.flush().await.map_err(|e| e.to_string())?;
    if expected_sha1
        .is_some_and(|expected| !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected))
    {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err("资源文件 SHA-1 校验失败".into());
    }
    tokio::fs::rename(&temp, target)
        .await
        .map_err(|e| e.to_string())?;
    emit_resource_progress(app, id, downloaded, total, "资源文件下载完成");
    Ok(())
}

#[tauri::command]
pub async fn download_resource(
    app: tauri::AppHandle,
    project_id: String,
    version_id: String,
    project_type: String,
    destination: String,
) -> Result<ResourceInstallResult, String> {
    if project_type != "modpack" {
        return Err("空实例目前只支持直接下载整合包".into());
    }
    let root = PathBuf::from(destination);
    if !root.is_absolute() {
        return Err("请选择有效的下载文件夹".into());
    }
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("无法创建下载文件夹：{e}"))?;
    let http = client()?;
    if version_id.is_empty() {
        return Err("请选择要下载的整合包版本".into());
    }
    let version: ProjectVersion = http
        .get(format!("https://api.modrinth.com/v2/version/{version_id}"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if version.project_id != project_id {
        return Err("所选版本不属于当前整合包".into());
    }
    let file = version
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| version.files.first())
        .ok_or("整合包版本没有可下载文件")?;
    if !file.url.starts_with("https://")
        || PathBuf::from(&file.filename)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("整合包下载地址或文件名不安全".into());
    }
    let requested = root.join(&file.filename);
    let target = available_download_path(requested);
    download_to_file(
        &app,
        &http,
        &version_id,
        &file.url,
        &target,
        file.hashes.get("sha1"),
    )
    .await?;
    Ok(ResourceInstallResult {
        path: target.to_string_lossy().into_owned(),
        project_type,
    })
}

fn available_download_path(requested: PathBuf) -> PathBuf {
    if !requested.exists() {
        return requested;
    }
    let parent = requested
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""));
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("modpack");
    let extension = requested.extension().and_then(|value| value.to_str());
    for index in 2.. {
        let name = match extension {
            Some(extension) => format!("{stem} ({index}).{extension}"),
            None => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

#[tauri::command]
pub async fn install_resource(
    app: tauri::AppHandle,
    project_id: String,
    project_type: String,
    instance_path: String,
    game_version: String,
    loader: String,
) -> Result<ResourceInstallResult, String> {
    let root = PathBuf::from(instance_path);
    if !root.is_absolute()
        || !root
            .join("versions")
            .join(&game_version)
            .join(".verdant-installed")
            .is_file()
    {
        return Err("请先选择完整安装的目标实例".into());
    }
    let http = client()?;
    let mut request = http.get(format!(
        "https://api.modrinth.com/v2/project/{project_id}/version"
    ));
    request = request.query(&[(
        "game_versions",
        serde_json::to_string(&vec![&game_version]).unwrap(),
    )]);
    if project_type == "mod" && !loader.is_empty() {
        request = request.query(&[("loaders", serde_json::to_string(&vec![&loader]).unwrap())]);
    }
    let versions: Vec<ProjectVersion> = request
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let version = versions.first().ok_or("没有与当前实例兼容的资源版本")?;
    let file = version
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| version.files.first())
        .ok_or("资源版本没有可下载文件")?;
    if !file.url.starts_with("https://")
        || PathBuf::from(&file.filename)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("资源下载地址或文件名不安全".into());
    }
    let folder = match project_type.as_str() {
        "mod" => "mods",
        "shader" => "shaderpacks",
        "resourcepack" => "resourcepacks",
        "datapack" => "datapacks",
        "modpack" => ".catcl-packs",
        _ => return Err("不支持的资源类型".into()),
    };
    let target = root.join("game").join(folder).join(&file.filename);
    tokio::fs::create_dir_all(target.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    download_to_file(
        &app,
        &http,
        &project_id,
        &file.url,
        &target,
        file.hashes.get("sha1"),
    )
    .await?;
    Ok(ResourceInstallResult {
        path: target.to_string_lossy().into_owned(),
        project_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_modrinth_project_id_and_chinese_aliases() {
        let response: SearchResponse = serde_json::from_value(serde_json::json!({"hits":[{"project_id":"AABB","title":"Sodium","description":"Fast renderer","project_type":"mod","icon_url":null,"downloads":10,"author":"jellysquid"}]})).unwrap();
        assert_eq!(response.hits[0].id, "AABB");
        assert_eq!(chinese_alias("暮色森林模组"), Some("twilight forest"));
        assert_eq!(chinese_alias("unknown"), None);
        assert_eq!(encode_query("JEI 中文"), "JEI%20%E4%B8%AD%E6%96%87");
    }

    #[test]
    fn keeps_existing_downloads() {
        let root = std::env::temp_dir().join(format!("catcl-resource-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("pack.mrpack");
        std::fs::write(&original, b"existing").unwrap();
        assert_eq!(
            available_download_path(original),
            root.join("pack (2).mrpack")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
