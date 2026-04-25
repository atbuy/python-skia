use std::io;

use anyhow::Context;
use skia_recorder::{RecorderDaemon, run_jsonl};
use tracing_subscriber::{EnvFilter, fmt};

fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(io::stderr)
        .init();

    let stdin = io::stdin();
    let stdout = io::stdout();

    run_jsonl(stdin.lock(), stdout.lock(), RecorderDaemon::new())
        .context("recorder JSONL loop failed")
}
