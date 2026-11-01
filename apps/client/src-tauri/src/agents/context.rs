use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub home: PathBuf,
    pub app_data: PathBuf,
}

impl AgentContext {
    pub fn new(home: PathBuf, app_data: PathBuf) -> Self {
        Self { home, app_data }
    }

    pub fn managed_dir(&self) -> PathBuf {
        self.app_data.join("managed")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.managed_dir().join("backups")
    }

    pub fn home_path(&self, relative: &str) -> PathBuf {
        self.home.join(relative)
    }

    pub fn managed_path(&self, name: &str) -> PathBuf {
        self.managed_dir().join(name)
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}
