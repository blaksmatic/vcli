//! One module per top-level subcommand. Each exposes an `async fn run(...)`
//! that takes the global flags (`socket`, `output`) plus its typed args and
//! returns `CliResult<()>`.

pub mod health;
