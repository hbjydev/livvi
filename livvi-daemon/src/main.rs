use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    tracing::info!("Starting Livvi...");

    tokio::select! {
        _ = shutdown_signal() => {
            tracing::info!("Shutdown signal received, terminating...");
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(sigterm) = sigterm.as_mut() {
                    sigterm.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }
}
