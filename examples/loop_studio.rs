//! Compatibility launcher: Loop Studio uses the same shell as every GUI lab.
fn main() -> anyhow::Result<()> {
    gooey::gui::run(Some("Loop Studio"))
}
