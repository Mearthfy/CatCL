use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct LaunchSnapshot {
    pub schema: u32,
    pub generation: u64,
    pub java_path: String,
    pub working_directory: String,
    pub arguments: Vec<String>,
    pub classpath_entries: usize,
    pub main_class: String,
}
