//! Reading project files that may be anything: a named pipe called
//! `lombok.config` or a classpath list pointing at `/dev/zero` would block or
//! never end a plain `read_to_string`.

use std::io;
use std::path::Path;

/// The text of a regular file (following symlinks); an error for a named
/// pipe, a device, a socket or a directory.
pub fn read_text(path: impl AsRef<Path>) -> io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let path = path.as_ref();
    // opened without blocking (a pipe with no writer would block in open),
    // then checked and read through the same handle
    let mut f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    if !f.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a regular file", path.display())));
    }
    let mut text = String::new();
    f.read_to_string(&mut text)?;
    Ok(text)
}

/// The bytes of a regular file, refused as `read_text` refuses (for hashing
/// what may not be text: a jar on the classpath).
pub fn read_bytes(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let path = path.as_ref();
    let mut f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    if !f.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a regular file", path.display())));
    }
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Write `bytes` to `path` whole: into a temporary file beside it, named for
/// this process and this write, then renamed over it. A reader, or a crash,
/// sees the old content or the new and never a part of either; two writers
/// at once (the daemon and an in-process fallback saving the stub cache)
/// each rename a file of their own, and the last rename wins.
pub fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let written = std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::{read_bytes, read_text};

    #[test]
    fn only_regular_files_are_read() {
        let dir = std::env::temp_dir().join(format!("grounds-fsutil-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        assert_eq!(read_text(dir.join("a.txt")).unwrap(), "x");
        // found by scripts/fuzz_robust.py: a named pipe blocked the whole run
        let fifo = dir.join("lombok.config");
        let ok = std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success());
        if ok {
            assert!(read_text(&fifo).is_err());
        }
        assert!(read_text("/dev/zero").is_err());
        assert!(read_text(&dir).is_err());
        assert!(read_bytes("/dev/zero").is_err());
        assert!(read_bytes(&dir).is_err());
        assert_eq!(read_bytes(dir.join("a.txt")).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
