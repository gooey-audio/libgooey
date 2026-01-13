//! Logging utilities for examples and applications

/// Initialize the logger with default settings for terminal applications.
/// Uses INFO level by default, with a format that works correctly in raw terminal mode.
/// The RUST_LOG environment variable can override the default level.
pub fn init_logger() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "\r[{} {:5} {}] {}",
                buf.timestamp(),
                record.level(),
                record.module_path().unwrap_or("unknown"),
                record.args()
            )
        })
        .init();
}
