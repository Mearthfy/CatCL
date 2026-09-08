use super::timeline::AnalysisSummary;
use crate::launch::session::LaunchMetrics;
use serde::{Deserialize, Serialize};
use std::path::Path;

const HISTORY_LIMIT: usize = 20;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkRecord {
    session_id: String,
    version: String,
    total_startup_ns: u64,
    ccl_preparation_ns: u64,
    jvm_startup_ns: Option<u64>,
    loader_phase_ns: Option<u64>,
    resource_reload_ns: Option<u64>,
    stall_count: usize,
    cache_hit_rate: f32,
    hotset_status: String,
    cds_status: String,
    java: String,
    ccl_version: String,
    instance_generation: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkHistory {
    schema: u32,
    records: Vec<BenchmarkRecord>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Comparison<'a> {
    baseline: &'a BenchmarkRecord,
    previous: Option<&'a BenchmarkRecord>,
    current: &'a BenchmarkRecord,
    best: &'a BenchmarkRecord,
}

pub async fn append(
    instance_root: &Path,
    session_id: &str,
    version: &str,
    metrics: &LaunchMetrics,
    analysis: &AnalysisSummary,
) -> Result<(), String> {
    let directory = instance_root.join(".catcl");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|e| e.to_string())?;
    let path = directory.join("benchmarks.json");
    let mut history: BenchmarkHistory = tokio::fs::read(&path)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    history.schema = 1;
    let ready = analysis.main_menu_ready_ns.unwrap_or(analysis.total_ns);
    history.records.push(BenchmarkRecord {
        session_id: session_id.into(),
        version: version.into(),
        total_startup_ns: ready,
        ccl_preparation_ns: metrics.ccl_preparation_ns,
        jvm_startup_ns: analysis.first_jvm_output_ns,
        loader_phase_ns: analysis
            .mixin_start_ns
            .zip(analysis.loader_start_ns)
            .map(|(end, start)| end.saturating_sub(start)),
        resource_reload_ns: analysis
            .resource_reload_start_ns
            .map(|start| ready.saturating_sub(start)),
        stall_count: analysis.stall_count,
        cache_hit_rate: metrics.cache_hit_rate,
        hotset_status: metrics.hotset_status.clone(),
        cds_status: "disabled".into(),
        java: metrics.java.clone(),
        ccl_version: env!("CARGO_PKG_VERSION").into(),
        instance_generation: metrics.instance_generation,
    });
    if let (Some(current), Some(previous)) =
        (history.records.last(), history.records.iter().rev().nth(1))
    {
        if current.hotset_status == "active"
            && current.total_startup_ns > previous.total_startup_ns.saturating_mul(105) / 100
        {
            let _ = tokio::fs::write(
                directory.join("hotset-disabled"),
                b"Automatically disabled after >5% startup regression",
            )
            .await;
        }
    }
    if history.records.len() > HISTORY_LIMIT {
        history
            .records
            .drain(..history.records.len() - HISTORY_LIMIT);
    }
    let bytes = serde_json::to_vec_pretty(&history).map_err(|e| e.to_string())?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| e.to_string())?;
    let current = history.records.last().unwrap();
    let comparison = Comparison {
        baseline: history.records.first().unwrap(),
        previous: history.records.iter().rev().nth(1),
        current,
        best: history
            .records
            .iter()
            .min_by_key(|item| item.total_startup_ns)
            .unwrap(),
    };
    let bytes = serde_json::to_vec_pretty(&comparison).map_err(|e| e.to_string())?;
    tokio::fs::write(directory.join("benchmark-summary.json"), bytes)
        .await
        .map_err(|e| e.to_string())
}
