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

#[cfg(test)]
mod tests {
    use super::read_text;

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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
