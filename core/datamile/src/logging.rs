use crate::compile::LogVerbosity;

pub type LogTx = tokio::sync::mpsc::Sender<String>;

/// Emits one progress message to the log and to every stream subscriber.
/// Headless runs have no subscribers, so the message only reaches the log.
pub fn emit(msg: &str, verbosity: LogVerbosity, txs: &[LogTx]) {
    if verbosity == LogVerbosity::Verbose {
        log::info!("{msg}");
    }

    for tx in txs {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
            tx.try_send(msg.to_string())
        {
            log::warn!(
                "Update log receiver queue full; dropping message for stream subscriber."
            );
        }
    }
}
