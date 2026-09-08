use std::time::{Duration, Instant};

pub struct LaunchTrace {
    started: Instant,
    last: Instant,
    stages: Vec<(String, Duration)>,
    skipped: Vec<String>,
}

impl LaunchTrace {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last: now,
            stages: Vec::new(),
            skipped: Vec::new(),
        }
    }
    pub fn mark(&mut self, name: impl Into<String>) {
        let now = Instant::now();
        self.stages
            .push((name.into(), now.duration_since(self.last)));
        self.last = now;
    }
    pub fn record(&mut self, name: impl Into<String>, elapsed: Duration) {
        self.stages.push((name.into(), elapsed));
    }
    pub fn elapsed_ns(&self) -> u64 {
        self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }
    pub fn skip(&mut self, name: impl Into<String>) {
        self.skipped.push(name.into());
    }
    pub fn report(&self, hit_rate: f32) -> String {
        let stages = self
            .stages
            .iter()
            .map(|(name, elapsed)| format!("{name}={}ms", elapsed.as_millis()))
            .collect::<Vec<_>>()
            .join(" · ");
        let skipped = if self.skipped.is_empty() {
            "无".into()
        } else {
            self.skipped.join("、")
        };
        format!(
            "CCL 启动准备 {}ms · 缓存命中率 {:.0}% · {} · 跳过：{}",
            self.started.elapsed().as_millis(),
            hit_rate * 100.0,
            stages,
            skipped
        )
    }
}
