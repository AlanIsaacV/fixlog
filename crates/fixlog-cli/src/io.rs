//! Tiny I/O helpers shared by the CLI commands.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::read::MultiGzDecoder;
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

/// An input the consolidated reader can consume: either a path on disk
/// (possibly `.gz`) or the process stdin (`-`).
#[derive(Debug, Clone)]
pub enum InputSource {
    File(PathBuf),
    Stdin,
}

impl InputSource {
    /// Parse one CLI token. `-` (a single dash) maps to [`InputSource::Stdin`];
    /// anything else is treated as a path.
    pub fn from_arg(raw: &str) -> Self {
        if raw == "-" {
            InputSource::Stdin
        } else {
            InputSource::File(PathBuf::from(raw))
        }
    }
}

/// Open `src` for sequential reading, transparently decompressing `.gz`
/// files. Uses [`MultiGzDecoder`] so concatenated gzip members (e.g.
/// `cat a.gz b.gz > combined.gz` or rolled rotation) are handled.
///
/// Returns a `Box<dyn BufRead>` so call sites do not need to be generic
/// over the underlying reader.
pub fn open_source(src: &InputSource) -> Result<Box<dyn BufRead>> {
    match src {
        InputSource::Stdin => {
            // Reuse the global stdin handle; lock for stable performance
            // on large piped inputs.
            let stdin = std::io::stdin();
            Ok(Box::new(BufReader::with_capacity(64 * 1024, stdin)))
        }
        InputSource::File(path) => {
            let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
            if is_gzip_path(path) {
                let dec = MultiGzDecoder::new(file);
                Ok(Box::new(BufReader::with_capacity(64 * 1024, dec)))
            } else {
                Ok(Box::new(BufReader::with_capacity(64 * 1024, file)))
            }
        }
    }
}

fn is_gzip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gz"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::{Read, Write};

    fn gzip_bytes(payload: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(payload).unwrap();
        enc.finish().unwrap()
    }

    fn slurp(src: &InputSource) -> Vec<u8> {
        let mut out = Vec::new();
        open_source(src).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn plain_and_gz_round_trip_identical_bytes() {
        let payload =
            b"8=FIX.4.4\x019=12\x0135=A\x0149=S\x0156=T\x0110=000\x01\nhello world\n".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("a.log");
        let gz = dir.path().join("a.log.gz");
        std::fs::write(&plain, &payload).unwrap();
        std::fs::write(&gz, gzip_bytes(&payload)).unwrap();

        assert_eq!(slurp(&InputSource::File(plain)), payload);
        assert_eq!(slurp(&InputSource::File(gz)), payload);
    }

    #[test]
    fn truncated_gz_emits_io_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let gz = dir.path().join("trunc.log.gz");
        // Write a valid gz header but truncate the deflate stream halfway.
        let mut bytes = gzip_bytes(b"some valid payload bytes that compress fine");
        bytes.truncate(bytes.len() / 2);
        std::fs::write(&gz, bytes).unwrap();

        let mut out = Vec::new();
        let res = open_source(&InputSource::File(gz))
            .unwrap()
            .read_to_end(&mut out);
        assert!(res.is_err(), "truncated gz must surface an io::Error");
    }

    #[test]
    fn from_arg_distinguishes_stdin_from_path() {
        match InputSource::from_arg("-") {
            InputSource::Stdin => {}
            _ => panic!("'-' should map to Stdin"),
        }
        match InputSource::from_arg("a/b.log.gz") {
            InputSource::File(p) => assert_eq!(p, PathBuf::from("a/b.log.gz")),
            _ => panic!("path should map to File"),
        }
    }
}
