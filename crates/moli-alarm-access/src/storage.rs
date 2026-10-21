//! Private bounded files; the service owns the only writer.
use crate::Error;
use serde::{Serialize, de::DeserializeOwned};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Storage {
    pub root: PathBuf,
}
impl Storage {
    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
    pub fn load<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, Error> {
        let path = self.path(name);
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::Unavailable),
        };
        if !meta.is_file() || meta.len() > MAX_BYTES as u64 {
            return Err(Error::Corrupt);
        }
        #[cfg(unix)]
        let file = {
            let fd = rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| Error::Corrupt)?;
            let file = File::from(fd);
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| Error::Unavailable)?;
            file
        };
        #[cfg(not(unix))]
        let file = File::open(&path).map_err(|_| Error::Unavailable)?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Unavailable)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| Error::Corrupt)
    }
    pub async fn write_async<T: Serialize + Send + 'static>(
        &self,
        name: &'static str,
        value: T,
    ) -> Result<(), Error> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || storage.write(name, &value))
            .await
            .map_err(|_| Error::Unavailable)?
    }
    pub fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), Error> {
        let bytes = serde_json::to_vec(value).map_err(|_| Error::Unavailable)?;
        if bytes.len() > MAX_BYTES {
            return Err(Error::Capacity);
        }
        fs::create_dir_all(&self.root).map_err(|_| Error::Unavailable)?;
        let target = self.path(name);
        if let Ok(meta) = fs::symlink_metadata(&target)
            && !meta.is_file()
        {
            return Err(Error::Corrupt);
        }
        let temporary = self.path(&format!(".alarm-{}.tmp", crate::crypto::random_id()?));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(|_| Error::Unavailable)?;
            file.write_all(&bytes).map_err(|_| Error::Unavailable)?;
            file.sync_all().map_err(|_| Error::Unavailable)?;
            drop(file);
            fs::rename(&temporary, &target).map_err(|_| Error::Unavailable)?;
            File::open(Path::new(&self.root))
                .and_then(|file| file.sync_all())
                .map_err(|_| Error::Unavailable)
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
}
