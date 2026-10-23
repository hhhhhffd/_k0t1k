use serde::{Serialize, Serializer};

/// Error returned from commands. It travels to the UI as a plain string: the message of the error
/// followed by the messages of its causes that it does not already contain.
#[derive(Debug)]
pub struct AppError(String);

pub type CommandResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<E: std::error::Error> From<E> for AppError {
    fn from(error: E) -> Self {
        let mut message = error.to_string();
        let mut cause = error.source();
        while let Some(source) = cause {
            let text = source.to_string();
            if !message.contains(&text) {
                message.push_str(": ");
                message.push_str(&text);
            }
            cause = source.source();
        }
        Self(message)
    }
}

/// Runs blocking core code (file I/O) off the async runtime.
pub async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> k0t1k_core::Result<T> + Send + 'static,
) -> CommandResult<T> {
    Ok(tokio::task::spawn_blocking(work).await??)
}

/// Runs a core future that rustc cannot prove `Send` (its streams capture higher-ranked lifetimes)
/// on a blocking thread, where it does not need to be.
pub async fn run_unsend<T, F>(make: impl FnOnce() -> F + Send + 'static) -> CommandResult<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = k0t1k_core::Result<T>>,
{
    let runtime = tokio::runtime::Handle::current();
    Ok(tokio::task::spawn_blocking(move || runtime.block_on(make())).await??)
}
