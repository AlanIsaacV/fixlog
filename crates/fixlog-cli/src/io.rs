//! Tiny I/O helpers shared by the CLI commands.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use memmap2::Mmap;

/// Memory-map a file read-only.
///
/// The mapping is held by the returned [`Mmap`]; dereferencing it yields a `&[u8]`. Files of
/// any size are fine because the OS pages bytes in lazily.
pub fn mmap_file(path: &Path) -> Result<Mmap> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    // SAFETY: we only ever hand out shared references to the mapping. Modifying the file
    // externally while we read it could surface inconsistent bytes, but the worst case for a
    // read-only inspection tool is a parse error that the caller already handles.
    let mmap =
        unsafe { Mmap::map(&file) }.with_context(|| format!("mmapping {}", path.display()))?;
    Ok(mmap)
}

/// Take the first `n` bytes of `bytes`, or all of them if shorter.
pub fn head(bytes: &[u8], n: usize) -> &[u8] {
    &bytes[..bytes.len().min(n)]
}
