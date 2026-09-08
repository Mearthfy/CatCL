use super::{benchmark, timeline};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

async fn create_log(directory: &Path, name: &str) -> Result<File, String> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join(name))
        .await
        .map_err(|error| error.to_string())
}

#[derive(Clone)]
pub struct StartupSession(Arc<SessionInner>);

struct SessionInner {
    id: String,
    version: String,
    pid: u32,
    started: Instant,
    started_unix_ms: u64,
    directory: PathBuf,
    timeline: Mutex<File>,
    stdout: Mutex<File>,
    stderr: Mutex<File>,
    first_output: AtomicBool,
    metrics: LaunchMetrics,
    jfr_path: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchMetrics {
    pub ccl_preparation_ns: u64,
    pub cache_hit_rate: f32,
    pub instance_generation: u64,
    pub java: String,
    pub hotset_status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummary<'a> {
    schema: u32,
    session_id: &'a str,
    version: &'a str,
    pid: u32,
    started_unix_ms: u64,
    total_ns: u64,
    exit_code: Option<i32>,
    timeline: &'static str,
    stdout: &'static str,
    stderr: &'static str,
    jfr: Option<String>,
}

impl StartupSession {
    pub async fn create(
        working_directory: &Path,
        version: &str,
        pid: u32,
        started: Instant,
        metrics: LaunchMetrics,
        jfr_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let id = format!("{started_unix_ms}-{pid}");
        let instance_root = working_directory.parent().unwrap_or(working_directory);
        let directory = instance_root.join(".catcl/startup").join(&id);
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| error.to_string())?;
        let timeline = create_log(&directory, "timeline.log").await?;
        let stdout = create_log(&directory, "stdout.log").await?;
        let stderr = create_log(&directory, "stderr.log").await?;
        Ok(Self(Arc::new(SessionInner {
            id,
            version: version.into(),
            pid,
            started,
            started_unix_ms,
            directory,
            timeline: Mutex::new(timeline),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            first_output: AtomicBool::new(false),
            metrics,
            jfr_path,
        })))
    }

    pub fn id(&self) -> &str {
        &self.0.id
    }

    pub fn elapsed_ns(&self) -> u64 {
        self.0.started.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    async fn append(file: &Mutex<File>, bytes: &[u8]) {
        let mut file = file.lock().await;
        let _ = file.write_all(bytes).await;
    }

    pub async fn event(&self, event: &str, stream: &str, message: &str) {
        let line = format!(
            "{}\t{}\t{}\t{}\n",
            self.elapsed_ns(),
            event,
            stream,
            message.replace(['\r', '\n'], " ")
        );
        Self::append(&self.0.timeline, line.as_bytes()).await;
    }

    pub async fn output(&self, stream: &str, line: &str) {
        let raw = format!("{line}\n");
        if stream == "stderr" {
            Self::append(&self.0.stderr, raw.as_bytes()).await;
        } else {
            Self::append(&self.0.stdout, raw.as_bytes()).await;
        }
        if !self.0.first_output.swap(true, Ordering::AcqRel) {
            self.event("First JVM Output", stream, line).await;
        }
        self.event("JVM Output", stream, line).await;
    }

    pub async fn finish(&self, exit_code: Option<i32>) {
        self.event(
            "Process Exit",
            "process",
            &format!("exit_code={exit_code:?}"),
        )
        .await;
        for file in [&self.0.timeline, &self.0.stdout, &self.0.stderr] {
            let mut file = file.lock().await;
            let _ = file.flush().await;
            let _ = file.sync_all().await;
        }
        let summary = SessionSummary {
            schema: 1,
            session_id: &self.0.id,
            version: &self.0.version,
            pid: self.0.pid,
            started_unix_ms: self.0.started_unix_ms,
            total_ns: self.elapsed_ns(),
            exit_code,
            timeline: "timeline.log",
            stdout: "stdout.log",
            stderr: "stderr.log",
            jfr: self
                .0
                .jfr_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&summary) {
            let _ = tokio::fs::write(self.0.directory.join("session.json"), bytes).await;
        }
        if let Ok(analysis) = timeline::analyze(&self.0.directory, self.elapsed_ns()).await {
            let _ = benchmark::append(
                self.0
                    .directory
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or(&self.0.directory),
                &self.0.id,
                &self.0.version,
                &self.0.metrics,
                &analysis,
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preserves_raw_output_and_writes_timed_session_files() {
        let root = std::env::temp_dir().join(format!(
            "catcl-session-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = root.join("game");
        tokio::fs::create_dir_all(&game).await.unwrap();
        let session = StartupSession::create(
            &game,
            "test",
            42,
            Instant::now(),
            LaunchMetrics {
                ccl_preparation_ns: 1,
                cache_hit_rate: 1.0,
                instance_generation: 1,
                java: "java".into(),
                hotset_status: "disabled".into(),
            },
            None,
        )
        .await
        .unwrap();
        session.event("Process Spawn", "process", "created").await;
        session.output("stdout", "hello").await;
        session.output("stderr", "warning").await;
        session.finish(Some(0)).await;
        let directory = root.join(".catcl/startup").join(session.id());
        assert_eq!(
            tokio::fs::read_to_string(directory.join("stdout.log"))
                .await
                .unwrap(),
            "hello\n"
        );
        assert_eq!(
            tokio::fs::read_to_string(directory.join("stderr.log"))
                .await
                .unwrap(),
            "warning\n"
        );
        let timeline = tokio::fs::read_to_string(directory.join("timeline.log"))
            .await
            .unwrap();
        assert!(timeline.contains("\tProcess Spawn\t"));
        assert!(timeline.contains("\tFirst JVM Output\t"));
        assert!(timeline.contains("\tProcess Exit\t"));
        let summary: serde_json::Value = serde_json::from_slice(
            &tokio::fs::read(directory.join("session.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(summary["exitCode"], 0);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
