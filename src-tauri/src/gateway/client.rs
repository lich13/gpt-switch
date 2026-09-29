use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientId {
    #[default]
    Codex,
    Claude,
}
impl ClientId {
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }
    pub fn port(self) -> u16 {
        match self {
            Self::Codex => 15722,
            Self::Claude => 15723,
        }
    }
    pub fn address(self, port: u16) -> String {
        format!(
            "http://127.0.0.1:{port}{}",
            if self == Self::Codex { "/v1" } else { "" }
        )
    }
    pub fn config(self, home: &Path) -> PathBuf {
        home.join(if self == Self::Codex {
            "config.toml"
        } else {
            "settings.json"
        })
    }
}

pub fn claude_home() -> String {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
        .to_string_lossy()
        .into_owned()
}
