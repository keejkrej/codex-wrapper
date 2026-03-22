use std::env;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub app_name: String,
    pub cwd: PathBuf,
    pub state_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub auth_token: Option<String>,
    pub dev_url: Option<String>,
    pub static_dir: Option<PathBuf>,
}

impl ServerConfig {
    pub fn desktop(app_name: impl Into<String>, cwd: PathBuf) -> Self {
        Self {
            app_name: app_name.into(),
            state_dir: cwd.join(".t3code"),
            cwd,
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: None,
            dev_url: None,
            static_dir: None,
        }
    }

    pub fn from_env() -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let app_name = env::var("T3CODE_APP_NAME").unwrap_or_else(|_| "T3 Code".to_string());
        let state_dir = env::var("T3CODE_STATE_DIR")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.join(".t3code"));
        let host = env::var("T3CODE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = env::var("T3CODE_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(0);
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

        Self {
            app_name,
            cwd,
            state_dir,
            host,
            port,
            auth_token,
            dev_url,
            static_dir,
        }
    }
}
