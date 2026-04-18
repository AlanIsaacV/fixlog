#![allow(unsafe_code)]

//! I/O helpers. Duplicated from `fixlog-cli/src/io.rs` rather than re-exported
//! to keep the `fixlog-cli` → `fixlog-tui` dependency direction simple (the
//! CLI will depend on this crate for its `tui` subcommand, not the other
//! way round). If a third consumer appears, move this into `fixlog-core`.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;

/// Memory-map a file read-only.
pub fn mmap_file(path: &Path) -> Result<Mmap> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    // SAFETY: we only ever hand out shared references to the mapping. External mutations
    // of the file while we read may surface inconsistent bytes; the parser tolerates
    // corrupted messages via `tracing::warn!` + skip, never UB.
    let mmap =
        unsafe { Mmap::map(&file) }.with_context(|| format!("mmapping {}", path.display()))?;
    Ok(mmap)
}

/// Take the first `n` bytes of `bytes`, or all of them if shorter.
pub fn head(bytes: &[u8], n: usize) -> &[u8] {
    &bytes[..bytes.len().min(n)]
}
