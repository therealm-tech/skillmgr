//! The signal an in-flight run stops on.

/// Complete on `SIGTERM` or `SIGINT`.
///
/// # Panics
///
/// When the process cannot install a signal handler at all.
#[cfg(unix)]
pub async fn signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt()).expect("SIGINT handler");
    let mut terminate = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = interrupt.recv() => {},
        _ = terminate.recv() => {},
    }
}

/// Complete on the platform's interrupt.
///
/// # Panics
///
/// When the process cannot install a signal handler at all.
#[cfg(not(unix))]
pub async fn signal() {
    tokio::signal::ctrl_c().await.expect("interrupt handler");
}
