use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

const STALL_NS: u64 = 500_000_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogPoint {
    time_ns: u64,
    logger: String,
    thread: String,
    message: String,
    phase: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhaseMark {
    name: String,
    time_ns: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stall {
    severity: &'static str,
    start_time_ns: u64,
    end_time_ns: u64,
    duration_ns: u64,
    before_logger: String,
    before_message: String,
    after_logger: String,
    after_message: String,
    thread: String,
    phase: String,
    assessment: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Aggregate {
    key: String,
    stall_count: usize,
    total_stall_ns: u64,
    longest_stall_ns: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupTimeline {
    schema: u32,
    total_ns: u64,
    main_menu_ready_ns: Option<u64>,
    phases: Vec<PhaseMark>,
    stalls: Vec<Stall>,
    by_logger: Vec<Aggregate>,
    by_thread: Vec<Aggregate>,
    by_phase: Vec<Aggregate>,
}

pub struct AnalysisSummary {
    pub total_ns: u64,
    pub main_menu_ready_ns: Option<u64>,
    pub first_jvm_output_ns: Option<u64>,
    pub loader_start_ns: Option<u64>,
    pub mixin_start_ns: Option<u64>,
    pub resource_reload_start_ns: Option<u64>,
    pub stall_count: usize,
}

fn attribute(line: &str, name: &str) -> Option<String> {
    let start = line.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &line[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

fn clean_message(line: &str) -> Option<String> {
    if let Some(start) = line.find("<![CDATA[") {
        let value = &line[start + 9..];
        return Some(value.split("]]>").next().unwrap_or(value).trim().to_owned());
    }
    let value = line.trim();
    (!value.is_empty() && !value.starts_with("<log4j:") && !value.starts_with("</log4j:"))
        .then(|| value.to_owned())
}

fn phase_for(current: &str, logger: &str, thread: &str, message: &str) -> &'static str {
    let haystack = format!("{logger} {message}").to_ascii_lowercase();
    if haystack.contains("game took") || haystack.contains("main menu") {
        "Main Menu Ready"
    } else if haystack.contains("reloading resourcemanager") || haystack.contains("resource reload")
    {
        "Resource Reload"
    } else if thread.eq_ignore_ascii_case("Render thread") {
        "Render Thread"
    } else if haystack.contains("mixin") {
        "Mixin"
    } else if haystack.contains("loading minecraft") || haystack.contains("fabric loader") {
        "Loader"
    } else if current == "Process Spawn" || current == "First JVM Output" {
        "JVM Startup"
    } else {
        match current {
            "Loader" => "Loader",
            "Mixin" => "Mod Init",
            "Resource Reload" => "Resource Reload",
            "Render Thread" => "Render Thread",
            "Main Menu Ready" => "Main Menu Ready",
            _ => "Mod Init",
        }
    }
}

fn aggregate(stalls: &[Stall], select: impl Fn(&Stall) -> &str) -> Vec<Aggregate> {
    let mut values: BTreeMap<String, (usize, u64, u64)> = BTreeMap::new();
    for stall in stalls {
        let item = values.entry(select(stall).to_owned()).or_default();
        item.0 += 1;
        item.1 += stall.duration_ns;
        item.2 = item.2.max(stall.duration_ns);
    }
    values
        .into_iter()
        .map(
            |(key, (stall_count, total_stall_ns, longest_stall_ns))| Aggregate {
                key,
                stall_count,
                total_stall_ns,
                longest_stall_ns,
            },
        )
        .collect()
}

fn build(source: &str, total_ns: u64) -> StartupTimeline {
    let mut logger = String::new();
    let mut thread = String::new();
    let mut phase = "Process Spawn";
    let mut points = Vec::new();
    let mut phases = vec![PhaseMark {
        name: phase.into(),
        time_ns: 0,
    }];
    for line in source.lines() {
        let mut columns = line.splitn(4, '\t');
        let Some(time_ns) = columns.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let event = columns.next().unwrap_or_default();
        let _stream = columns.next().unwrap_or_default();
        let raw = columns.next().unwrap_or_default();
        if let Some(value) = attribute(raw, "logger") {
            logger = value;
        }
        if let Some(value) = attribute(raw, "thread") {
            thread = value;
        }
        if event == "First JVM Output" && phases.iter().all(|item| item.name != event) {
            phases.push(PhaseMark {
                name: event.into(),
                time_ns,
            });
        }
        let Some(message) = clean_message(raw) else {
            continue;
        };
        let next = phase_for(phase, &logger, &thread, &message);
        if next != phase {
            phase = next;
            phases.push(PhaseMark {
                name: phase.into(),
                time_ns,
            });
        }
        points.push(LogPoint {
            time_ns,
            logger: logger.clone(),
            thread: thread.clone(),
            message,
            phase: phase.into(),
        });
    }
    let mut stalls = Vec::new();
    for pair in points.windows(2) {
        let before = &pair[0];
        let after = &pair[1];
        let duration = after.time_ns.saturating_sub(before.time_ns);
        if duration < STALL_NS {
            continue;
        }
        stalls.push(Stall {
            severity: if duration >= 3_000_000_000 {
                "Critical Stall"
            } else if duration >= 1_000_000_000 {
                "Slow Stall"
            } else {
                "Stall"
            },
            start_time_ns: before.time_ns,
            end_time_ns: after.time_ns,
            duration_ns: duration,
            before_logger: before.logger.clone(),
            before_message: before.message.clone(),
            after_logger: after.logger.clone(),
            after_message: after.message.clone(),
            thread: if before.thread == after.thread {
                before.thread.clone()
            } else {
                format!("{} → {}", before.thread, after.thread)
            },
            phase: after.phase.clone(),
            assessment:
                "Suspicious time interval; adjacent loggers are context, not proven causes.",
        });
    }
    stalls.sort_by_key(|stall| std::cmp::Reverse(stall.duration_ns));
    let main_menu_ready_ns = phases
        .iter()
        .find(|item| item.name == "Main Menu Ready")
        .map(|item| item.time_ns);
    StartupTimeline {
        schema: 1,
        total_ns,
        main_menu_ready_ns,
        phases,
        by_logger: aggregate(&stalls, |item| &item.before_logger),
        by_thread: aggregate(&stalls, |item| &item.thread),
        by_phase: aggregate(&stalls, |item| &item.phase),
        stalls,
    }
}

pub async fn analyze(directory: &Path, total_ns: u64) -> Result<AnalysisSummary, String> {
    let source = tokio::fs::read_to_string(directory.join("timeline.log"))
        .await
        .map_err(|error| error.to_string())?;
    let report = build(&source, total_ns);
    let json = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    tokio::fs::write(directory.join("startup-report.json"), json)
        .await
        .map_err(|error| error.to_string())?;
    let longest = report
        .stalls
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, stall)| {
            format!(
                "{}. {:.3}s [{}] {}",
                index + 1,
                stall.duration_ns as f64 / 1e9,
                stall.phase,
                stall.assessment
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "CCL Startup Report\n\nTotal: {:.3}s\nMain Menu Ready: {}\n\nLongest stalls:\n{}\n",
        total_ns as f64 / 1e9,
        report
            .main_menu_ready_ns
            .map(|value| format!("{:.3}s", value as f64 / 1e9))
            .unwrap_or_else(|| "Not detected".into()),
        longest
    );
    tokio::fs::write(directory.join("startup-report.txt"), text)
        .await
        .map_err(|error| error.to_string())?;
    let phase = |name: &str| {
        report
            .phases
            .iter()
            .find(|item| item.name == name)
            .map(|item| item.time_ns)
    };
    Ok(AnalysisSummary {
        total_ns,
        main_menu_ready_ns: report.main_menu_ready_ns,
        first_jvm_output_ns: phase("First JVM Output"),
        loader_start_ns: phase("Loader"),
        mixin_start_ns: phase("Mixin"),
        resource_reload_start_ns: phase("Resource Reload"),
        stall_count: report.stalls.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_phases_and_context_only_stalls() {
        let source = "0\tProcess Spawn\tprocess\tcreated\n600000000\tJVM Output\tstdout\t<log4j:Event logger=\"FabricLoader/Mixin\" thread=\"main\">\n600100000\tJVM Output\tstdout\t<log4j:Message><![CDATA[SpongePowered MIXIN Subsystem]]></log4j:Message>\n4100100000\tJVM Output\tstdout\t<log4j:Event logger=\"ModernFix\" thread=\"Render thread\">\n4100200000\tJVM Output\tstdout\t<log4j:Message><![CDATA[Game took 4.1 seconds to start]]></log4j:Message>";
        let report = build(source, 4_200_000_000);
        assert_eq!(report.main_menu_ready_ns, Some(4_100_200_000));
        assert_eq!(report.stalls[0].severity, "Critical Stall");
        assert!(report.stalls[0].assessment.contains("not proven causes"));
    }
}
