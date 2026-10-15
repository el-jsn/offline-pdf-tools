use std::{fs::{self, File, OpenOptions}, path::{Path, PathBuf}};

// Only app-owned UUID directories are reclaimed. A held file lock protects live instances.
pub struct Workspace { directory: PathBuf, _lock: File }
impl Workspace {
    pub fn new() -> Result<Self, String> {
        let root = std::env::temp_dir().join("pdf-tools-private");
        fs::create_dir_all(&root).map_err(|_| "Cannot create private storage.")?;
        if fs::symlink_metadata(&root).map_err(|_| "Private storage unavailable.")?.file_type().is_symlink() {
            return Err("Private storage must not be a symbolic link.".into());
        }
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|_| "Cannot protect private storage.")?; }
        let sweep = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(root.join("sweep.lock")).map_err(|_| "Private storage unavailable.")?;
        sweep.lock().map_err(|_| "Private storage unavailable.")?;
        for entry in fs::read_dir(&root).map_err(|_| "Private storage unavailable.")?.flatten() {
            let path = entry.path();
            if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() || !entry.file_type().is_ok_and(|kind| kind.is_dir()) { continue; }
            if let Ok(lock) = OpenOptions::new().read(true).write(true).open(path.join("session.lock")) {
                if lock.try_lock().is_ok() { let _ = fs::remove_dir_all(path); }
            }
        }
        // The UUID directory appears without a lock file; the sweep leaves lock-free directories alone.
        let path = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&path).map_err(|_| "Cannot create private storage.")?;
        let lock = OpenOptions::new().create_new(true).read(true).write(true).open(path.join("session.lock")).map_err(|_| "Cannot protect private storage.")?;
        lock.lock().map_err(|_| "Cannot protect private storage.")?;
        Ok(Self { directory: path, _lock: lock })
    }
    pub fn path(&self) -> &Path { &self.directory }
    pub fn job(&self) -> Result<tempfile::TempDir, String> { tempfile::Builder::new().prefix("job-").tempdir_in(self.path()).map_err(|_| "Cannot create a working folder.".to_owned()) }
    pub fn cleanup(&self) { let _ = fs::remove_dir_all(&self.directory); }
}
impl Drop for Workspace {
    fn drop(&mut self) { self.cleanup(); }
}

pub fn safe_name(value: &str, fallback: &str) -> String {
    let cleaned: String = value.trim().chars().filter(|c| !c.is_control() && !"/\\:*?\"<>|".contains(*c)).take(80).collect();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() { fallback.to_owned() } else { cleaned.to_owned() }
}
