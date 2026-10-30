use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Retain the file even if the desktop launcher fails; the error includes its path.
pub fn show(html: &str, opener: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    let mut file = tempfile::Builder::new()
        .prefix("todo-unearther-")
        .suffix(".html")
        .tempfile()
        .context("could not create temporary HTML preview")?;
    file.write_all(html.as_bytes())
        .context("could not write HTML preview")?;
    file.flush()?;
    let (_, path) = file.keep().context("could not retain HTML preview")?;
    opener(&path).with_context(|| {
        format!(
            "could not open HTML preview; report retained at {}",
            path.display()
        )
    })?;
    Ok(path)
}
