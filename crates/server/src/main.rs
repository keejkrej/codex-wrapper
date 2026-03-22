use server::{start_server, ServerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut config = ServerConfig::from_env();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--app-name" => {
                if let Some(value) = args.next() {
                    config.app_name = value;
                }
            }
            "--cwd" => {
                if let Some(value) = args.next() {
                    config.cwd = value.into();
                }
            }
            "--host" => {
                if let Some(value) = args.next() {
                    config.host = value;
                }
            }
            "--port" => {
                if let Some(value) = args.next() {
                    if let Ok(port) = value.parse::<u16>() {
                        config.port = port;
                    }
                }
            }
            "--auth-token" => {
                if let Some(value) = args.next() {
                    config.auth_token = Some(value);
                }
            }
            "--dev-url" => {
                if let Some(value) = args.next() {
                    config.dev_url = Some(value);
                }
            }
            "--static-dir" => {
                if let Some(value) = args.next() {
                    config.static_dir = Some(value.into());
                }
            }
            _ => {}
        }
    }

    let handle = start_server(config).await?;
    println!("HTTP {}", handle.http_url());
    println!("WS   {}", handle.ws_url());

    tokio::signal::ctrl_c().await?;
    handle.shutdown().await;
    Ok(())
}
