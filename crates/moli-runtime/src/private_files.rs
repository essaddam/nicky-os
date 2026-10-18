//! Repair current public files before any reader or driver can observe them.
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};

/// No backup of unsanitized bytes. A failed write leaves the old authoritative
/// file in place and aborts startup, so repair is retried before serving anything.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".privacy.tmp");
    let tmp = Path::new(&tmp);
    let result = (|| {
        let mut file = File::create(tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(tmp, path)?;
        sync_parent(path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

/// An interrupted old writer's temporary file is not authoritative.
pub(crate) fn discard_pending(path: &Path) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    discard(Path::new(&tmp))
}

pub(crate) fn discard(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            sync_parent(path)?;
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}
