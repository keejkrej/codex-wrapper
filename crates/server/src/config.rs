use std::env;
use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 3773;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeMode {
    Web,
    Desktop,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub app_name: String,
    pub mode: RuntimeMode,
    pub cwd: PathBuf,
    pub state_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub auth_token: Option<String>,
    pub dev_url: Option<String>,
    pub static_dir: Option<PathBuf>,
    pub no_browser: bool,
    pub auto_bootstrap_project_from_cwd: bool,
    pub log_websocket_events: bool,
}

impl ServerConfig {
    pub fn desktop(app_name: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            app_name: app_name.into(),
            mode: RuntimeMode::Desktop,
            state_dir: cwd.join(".t3code"),
            cwd,
            host: "127.0.0.1".to_string(),
            port: DEFAULT_PORT,
            auth_token: None,
            dev_url: None,
            static_dir: None,
            no_browser: true,
            auto_bootstrap_project_from_cwd: false,
            log_websocket_events: false,
        }
    }

    pub fn from_env() -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let app_name = env::var("T3CODE_APP_NAME").unwrap_or_else(|_| "T3 Code".to_string());
        let mode = match env::var("T3CODE_MODE")
            .unwrap_or_else(|_| "web".to_string())
            .to_lowercase()
            .as_str()
        {
            "desktop" => RuntimeMode::Desktop,
            _ => RuntimeMode::Web,
        };
        let state_dir = env::var("T3CODE_STATE_DIR")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.join(".t3code"));
        let host = env::var("T3CODE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = env::var("T3CODE_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or_else(|| {
                if mode == RuntimeMode::Desktop {
                    DEFAULT_PORT
                } else {
                    0
                }
            });
        let auth_token = env::var("T3CODE_AUTH_TOKEN")
            .ok()
            .filter(|value| !value.is_empty());
        let dev_url = env::var("VITE_DEV_SERVER_URL")
            .ok()
            .or_else(|| env::var("T3CODE_DEV_URL").ok())
            .filter(|value| !value.is_empty());
        let static_dir = env::var("T3CODE_STATIC_DIR")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let no_browser = env::var("T3CODE_NO_BROWSER")
            .ok()
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
            .unwrap_or(mode == RuntimeMode::Desktop);
        let auto_bootstrap_project_from_cwd = env::var("T3CODE_AUTO_BOOTSTRAP_PROJECT_FROM_CWD")
            .ok()
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
            .unwrap_or(mode == RuntimeMode::Web);
        let log_websocket_events = env::var("T3CODE_LOG_WS_EVENTS")
            .ok()
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
            .unwrap_or(dev_url.is_some());

        Self {
            app_name,
            mode,
            cwd,
            state_dir,
            host,
            port,
            auth_token,
            dev_url,
            static_dir,
            no_browser,
            auto_bootstrap_project_from_cwd,
            log_websocket_events,
        }
    }
}
