mod api;
mod handler;

use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::info;

/// Unix socket path shared between the daemon and Swift client.
pub const SOCKET_PATH: &str = "/tmp/osmos-daemon.sock";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("osmos_daemon=debug".parse()?)
                .add_directive("osmos_core=debug".parse()?),
        )
        .init();

    // Remove stale socket from a previous run.
    let socket_path = Path::new(SOCKET_PATH);
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(SOCKET_PATH)?;
    info!("osmos-daemon listening on {SOCKET_PATH}");

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_connection(stream));
    }
}

/// Reads newline-delimited JSON requests from a client connection and writes responses.
async fn handle_connection(stream: UnixStream) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let response = match serde_json::from_str::<api::Request>(&line) {
            Ok(req) => {
                let id = req.id;
                // handler is synchronous (SQLite) — run on blocking thread pool.
                tokio::task::spawn_blocking(move || handler::handle(req))
                    .await
                    .unwrap_or_else(|_| api::Response::err(id, "INTERNAL", "task panicked"))
            }
            Err(e) => {
                // Can't echo the id back if JSON is malformed; use nil UUID.
                api::Response::err(uuid::Uuid::nil(), "PARSE_ERROR", e)
            }
        };

        let mut json = serde_json::to_string(&response).unwrap_or_default();
        json.push('\n');
        if writer.write_all(json.as_bytes()).await.is_err() {
            break;
        }
    }
}
