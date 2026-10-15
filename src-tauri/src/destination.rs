use std::{fs, path::{Path, PathBuf}};

// Remember the selected directory, including its filesystem. A removed volume
// can expose an ordinary directory at the same mountpoint during processing.
pub struct Destination {
    path: PathBuf,
    canonical: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl Destination {
    pub fn capture(path: &Path) -> Result<Self, String> {
        let canonical = fs::canonicalize(path).map_err(|_| "Cannot access the output folder.")?;
        let metadata = fs::metadata(path).map_err(|_| "Cannot access the output folder.")?;
        if !metadata.is_dir() { return Err("Choose an output folder.".into()); }
        #[cfg(unix)]
        let identity = { use std::os::unix::fs::MetadataExt; (metadata.dev(), metadata.ino()) };
        Ok(Self { path: path.to_owned(), canonical, #[cfg(unix)] identity })
    }

    pub fn verify(&self) -> Result<(), String> {
        let error = "The output folder was disconnected or changed. Choose it again.";
        let metadata = fs::metadata(&self.path).map_err(|_| error)?;
        if !metadata.is_dir() || fs::canonicalize(&self.path).map_err(|_| error)? != self.canonical {
            return Err(error.into());
        }
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            if (metadata.dev(), metadata.ino()) != self.identity { return Err(error.into()); }
        }
        Ok(())
    }
}
