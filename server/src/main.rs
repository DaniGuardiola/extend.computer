use extend_computer_server::{router, Config, Server};
use std::{net::SocketAddr, path::PathBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind: SocketAddr = std::env::var("EXTEND_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    let database =
        PathBuf::from(std::env::var("EXTEND_DATABASE").unwrap_or_else(|_| "extend.sqlite3".into()));
    if let Some(parent) = database.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let signup_enabled = match std::env::var("EXTEND_SIGNUP_ENABLED").as_deref() {
        Ok("true") => true,
        Ok("false") | Err(std::env::VarError::NotPresent) => false,
        _ => anyhow::bail!("EXTEND_SIGNUP_ENABLED must be true or false"),
    };
    let server = Server::open(
        &database,
        Config {
            signup_enabled,
            origin: std::env::var("EXTEND_ORIGIN").ok(),
        },
    )?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "extend.computer account server listening on {}",
        listener.local_addr()?
    );
    axum::serve(
        listener,
        router(server).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
