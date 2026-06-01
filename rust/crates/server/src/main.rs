use std::env;
use std::net::SocketAddr;

use server::{app, AppState};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = sanitize_env_value("CLOUD_CODE_HOST").unwrap_or_else(|| "127.0.0.1".to_string());
    let port = env::var("CLOUD_CODE_PORT")
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8787);
    let address: SocketAddr = format!("{host}:{port}").parse()?;

    let listener = TcpListener::bind(address).await?;
    println!("cloud-code listening on http://{address}");

    axum::serve(listener, app(AppState::default())).await?;
    Ok(())
}

fn sanitize_env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}
