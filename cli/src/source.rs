use crate::editor::{EditorDocument, MAX_DOCUMENT_BYTES};
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

fn c_string(value: &OsStr) -> Result<CString, String> {
    CString::new(value.as_bytes()).map_err(|_| "source path contains a NUL byte".into())
}

fn openat2(directory: &OwnedFd, path: &Path, flags: i32, mode: u32) -> Result<OwnedFd, String> {
    assert!(flags >= 0);
    let path = c_string(path.as_os_str())?;
    let how = OpenHow {
        flags: flags as u64,
        mode: u64::from(mode),
        resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    };
    // SAFETY: pointers reference initialized values for the duration of the syscall; the returned
    // descriptor is uniquely owned when nonnegative.
    let descriptor = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            directory.as_raw_fd(),
            path.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let descriptor =
        i32::try_from(descriptor).map_err(|_| "openat2 returned invalid descriptor".to_string())?;
    // SAFETY: openat2 returned a new descriptor and ownership transfers here exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

fn open_root(root: &Path) -> Result<(PathBuf, OwnedFd), String> {
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize project root: {error}"))?;
    let path = c_string(canonical.as_os_str())?;
    // SAFETY: path is NUL terminated and flags require no variadic mode argument.
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "cannot anchor project root: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: libc::open returned a new descriptor and ownership transfers exactly once.
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    Ok((canonical, descriptor))
}

fn relative_path(canonical_root: &Path, planned_path: &Path) -> Result<PathBuf, String> {
    let relative = if planned_path.is_absolute() {
        planned_path
            .strip_prefix(canonical_root)
            .map_err(|_| "solution path escapes project root".to_string())?
    } else {
        planned_path
    };
    if relative.as_os_str().is_empty() {
        return Err("solution path must name a file".into());
    }
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err("solution path contains an unsafe component".into());
        }
    }
    Ok(relative.to_path_buf())
}

fn regular_metadata(file: &OwnedFd, context: &str) -> Result<libc::stat, String> {
    // SAFETY: zeroed stat is a valid output buffer and fd remains open through fstat.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: stat points to writable initialized storage.
    if unsafe { libc::fstat(file.as_raw_fd(), &mut stat) } != 0 {
        return Err(format!(
            "cannot inspect {context}: {}",
            std::io::Error::last_os_error()
        ));
    }
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(format!("{context} must be a regular file"));
    }
    Ok(stat)
}

fn anchored(root: &Path, planned_path: &Path) -> Result<(OwnedFd, PathBuf), String> {
    let (canonical, root_fd) = open_root(root)?;
    let relative = relative_path(&canonical, planned_path)?;
    Ok((root_fd, relative))
}

pub fn load(root: &Path, planned_path: &Path) -> Result<EditorDocument, String> {
    let (root_fd, relative) = anchored(root, planned_path)?;
    let descriptor = openat2(
        &root_fd,
        &relative,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK,
        0,
    )
    .map_err(|error| format!("cannot safely open solution file: {error}"))?;
    let metadata = regular_metadata(&descriptor, "solution file")?;
    if metadata.st_size < 0 || metadata.st_size as u64 > MAX_DOCUMENT_BYTES as u64 {
        return Err(format!("solution file exceeds {MAX_DOCUMENT_BYTES} bytes"));
    }
    let capacity =
        usize::try_from(metadata.st_size).map_err(|_| "solution size is invalid".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut file = File::from(descriptor);
    Read::by_ref(&mut file)
        .take((MAX_DOCUMENT_BYTES + 1) as u64)
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
    let (root_fd, relative) = anchored(root, planned_path)?;
    let parent_relative = relative
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = relative.file_name().ok_or("solution file has no name")?;
    let parent_fd = openat2(
        &root_fd,
        parent_relative,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        0,
    )
    .map_err(|error| format!("cannot safely open solution parent: {error}"))?;
    let existing = openat2(
        &parent_fd,
        Path::new(file_name),
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK,
        0,
    )
    .map_err(|error| format!("cannot safely open solution target: {error}"))?;
    let metadata = regular_metadata(&existing, "solution target")?;
    let mode = metadata.st_mode & 0o7777;

    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_name = format!(".interview-save-{}-{sequence}", std::process::id());
    let temporary_path = Path::new(&temporary_name);
    let temporary_fd = openat2(
        &parent_fd,
        temporary_path,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
        0o600,
    )
    .map_err(|error| format!("cannot create temporary source: {error}"))?;
    let cleanup = || {
        let Ok(name) = c_string(temporary_path.as_os_str()) else {
            return;
        };
        // SAFETY: name is NUL terminated and parent_fd remains open.
        unsafe {
            libc::unlinkat(parent_fd.as_raw_fd(), name.as_ptr(), 0);
        }
    };

    let result = (|| {
        let mut temporary = File::from(temporary_fd);
        temporary
            .write_all(text.as_bytes())
            .map_err(|error| format!("cannot write temporary source: {error}"))?;
        temporary
            .flush()
            .map_err(|error| format!("cannot flush temporary source: {error}"))?;
        // SAFETY: descriptor is open and mode is restricted to permission/special bits from fstat.
        if unsafe { libc::fchmod(temporary.as_raw_fd(), mode as libc::mode_t) } != 0 {
            return Err(format!(
                "cannot preserve source mode: {}",
                std::io::Error::last_os_error()
            ));
        }
        temporary
            .sync_all()
            .map_err(|error| format!("cannot sync temporary source: {error}"))?;
        before_rename(temporary_path)?;
        let from = c_string(temporary_path.as_os_str())?;
        let to = c_string(file_name)?;
        // SAFETY: names are NUL terminated and both operations are anchored to the same open parent.
        if unsafe {
            libc::renameat(
                parent_fd.as_raw_fd(),
                from.as_ptr(),
                parent_fd.as_raw_fd(),
                to.as_ptr(),
            )
        } != 0
        {
            return Err(format!(
                "cannot replace source file: {}",
                std::io::Error::last_os_error()
            ));
        }
        // Durability order is data+mode sync, anchored rename, then directory sync.
        // SAFETY: parent_fd is an open directory descriptor.
        if unsafe { libc::fsync(parent_fd.as_raw_fd()) } != 0 {
            return Err(format!(
                "cannot sync source directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        cleanup();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
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
    fn exact_bytes_mode_failure_cleanup_and_preservation() {
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
        assert!(fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".interview-save-")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_final_and_ancestor_symlinks_and_fifo_without_blocking() {
        let root = fixture();
        fs::create_dir(root.join("real")).unwrap();
        fs::write(root.join("real/answer.py"), b"x").unwrap();
        std::os::unix::fs::symlink("real", root.join("alias")).unwrap();
        assert!(load(&root, &root.join("alias/answer.py")).is_err());
        std::os::unix::fs::symlink("real/answer.py", root.join("link.py")).unwrap();
        assert!(load(&root, &root.join("link.py")).is_err());
        let fifo_path = root.join("pipe");
        let fifo_c = c_string(fifo_path.as_os_str()).unwrap();
        // SAFETY: path is NUL terminated and does not exist.
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        assert!(
            load(&root, &fifo_path)
                .unwrap_err()
                .contains("regular file")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn anchored_parent_survives_ancestor_swap_before_rename() {
        let root = fixture();
        fs::create_dir(root.join("source")).unwrap();
        fs::write(root.join("source/answer.py"), b"old").unwrap();
        let path = root.join("source/answer.py");
        atomic_save_with(&root, &path, "new", |_| {
            fs::rename(root.join("source"), root.join("moved")).unwrap();
            fs::create_dir(root.join("source")).unwrap();
            fs::write(root.join("source/answer.py"), b"decoy").unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(root.join("moved/answer.py")).unwrap(), b"new");
        assert_eq!(fs::read(root.join("source/answer.py")).unwrap(), b"decoy");
        fs::remove_dir_all(root).unwrap();
    }
}
