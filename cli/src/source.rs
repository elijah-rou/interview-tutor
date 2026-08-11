use crate::editor::{EditorDocument, MAX_DOCUMENT_BYTES};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn checked_paths(root: &Path, planned_path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize project root: {error}"))?;
    if !root.is_dir() {
        return Err("project root is not a directory".into());
    }
    let candidate = if planned_path.is_absolute() {
        planned_path.to_path_buf()
    } else {
        root.join(planned_path)
    };
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        format!(
            "cannot inspect solution file {}: {error}",
            candidate.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err("solution path must not be a symlink".into());
    }
    if !metadata.is_file() {
        return Err("solution path must be a regular file".into());
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize solution file: {error}"))?;
    if !canonical.starts_with(&root) {
        return Err("solution path escapes project root".into());
    }
    Ok((root, canonical))
}

pub fn load(root: &Path, planned_path: &Path) -> Result<EditorDocument, String> {
    let (_, path) = checked_paths(root, planned_path)?;
    let metadata =
        fs::metadata(&path).map_err(|error| format!("cannot inspect solution file: {error}"))?;
    if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(format!("solution file exceeds {MAX_DOCUMENT_BYTES} bytes"));
    }
    let file = File::open(&path).map_err(|error| format!("cannot open solution file: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read solution file: {error}"))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(format!("solution file exceeds {MAX_DOCUMENT_BYTES} bytes"));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| "solution file is not valid UTF-8".to_string())?;
    EditorDocument::new(text)
}

pub fn atomic_save(root: &Path, planned_path: &Path, text: &str) -> Result<(), String> {
    atomic_save_with(root, planned_path, text, |_| Ok(()))
}

pub fn atomic_save_with(
    root: &Path,
    planned_path: &Path,
    text: &str,
    before_rename: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    EditorDocument::new(text.to_string())?;
    let (_, path) = checked_paths(root, planned_path)?;
    let parent = path
        .parent()
        .ok_or("solution file has no parent directory")?;
    let mode = fs::metadata(&path)
        .map_err(|error| format!("cannot inspect solution mode: {error}"))?
        .permissions()
        .mode();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".interview-save-{}-{sequence}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|error| format!("cannot create temporary source: {error}"))?;
        file.write_all(text.as_bytes())
            .map_err(|error| format!("cannot write temporary source: {error}"))?;
        file.flush()
            .map_err(|error| format!("cannot flush temporary source: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync temporary source: {error}"))?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("cannot preserve source mode: {error}"))?;
        before_rename(&temporary)?;
        fs::rename(&temporary, &path)
            .map_err(|error| format!("cannot replace source file: {error}"))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("cannot sync source directory: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "source-save-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }
    #[test]
    fn exact_bytes_mode_and_failure_preservation() {
        let root = fixture();
        let path = root.join("answer.py");
        fs::write(&path, b"old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o754)).unwrap();
        atomic_save(&root, &path, "界\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), "界\n".as_bytes());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o754
        );
        let error = atomic_save_with(&root, &path, "new", |_| Err("injected".into())).unwrap_err();
        assert_eq!(error, "injected");
        assert_eq!(fs::read(&path).unwrap(), "界\n".as_bytes());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_symlinks_and_invalid_utf8() {
        let root = fixture();
        let outside = root
            .parent()
            .unwrap()
            .join(format!("outside-{}", std::process::id()));
        fs::write(&outside, b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(load(&root, &root.join("link")).is_err());
        fs::write(root.join("bad"), [0xff]).unwrap();
        assert!(load(&root, &root.join("bad")).is_err());
        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
