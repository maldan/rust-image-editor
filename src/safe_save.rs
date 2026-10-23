use std::{
    collections::hash_map::DefaultHasher,
    fs::OpenOptions,
    hash::{Hash, Hasher},
    path::Path,
};

use mega_file::{MegaFile, WriteFile};

/// ReplaceFile preserves readers of the previous revision on Windows, unlike
/// MoveFileEx (used by TempPath::persist) when the destination is still open.
#[cfg(windows)]
fn replace_open_file(temporary: &Path, path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let backup = tempfile::Builder::new()
        .prefix(".mgimg-backup-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|e| e.to_string())?
        .into_temp_path();
    std::fs::remove_file(&backup).map_err(|e| e.to_string())?;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<u16>>()
    };
    let replaced = wide(path);
    let replacement = wide(temporary);
    let backup_name = wide(&backup);
    // SAFETY: all paths are NUL-terminated UTF-16 and remain alive for the call;
    // the reserved pointers are null, as required by ReplaceFileW.
    let ok = unsafe {
        ReplaceFileW(
            replaced.as_ptr(),
            replacement.as_ptr(),
            backup_name.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let error = std::io::Error::last_os_error();
        if backup.exists() {
            if !path.exists() && std::fs::rename(&backup, path).is_ok() {
                return Err(format!(
                    "Cannot replace project; previous file was restored: {error}"
                ));
            }
            let recovery = backup.keep().map_err(|e| e.to_string())?;
            return Err(format!(
                "Cannot replace project: {error}; previous file retained at {}",
                recovery.display()
            ));
        }
        return Err(format!(
            "Cannot replace project; previous file was kept: {error}"
        ));
    }
    Ok(())
}

/// Prepare and verify a sibling file before atomically replacing the destination.
/// Never remove or truncate the destination, including when replacement fails.
fn replace_file(
    path: &Path,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".mgimg-save-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|e| format!("Cannot create temporary project: {e}"))?
        .into_temp_path();
    prepare(&temporary)?;
    OpenOptions::new()
        .write(true)
        .open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("Cannot sync temporary project: {e}"))?;
    #[cfg(windows)]
    if path.exists() {
        return replace_open_file(&temporary, path);
    }
    temporary
        .persist(path)
        .map_err(|e| format!("Cannot replace project; previous file was kept: {e}"))?;
    Ok(())
}

pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    hash.finish()
}

/// Append all changed resources and their manifest in a single archive snapshot.
pub fn append_archive(
    path: &Path,
    files: Vec<WriteFile>,
    expected_manifest: Option<&[u8]>,
) -> Result<(), String> {
    let archive = MegaFile::open(path)
        .map_err(|e| format!("Cannot open project for incremental save: {e}"))?;
    if let Some(expected) = expected_manifest {
        let actual = archive
            .read_all("/manifest.json")
            .map_err(|e| format!("Cannot check saved project: {e}"))?;
        let matches_pending = files
            .iter()
            .any(|file| file.path == "/manifest.json" && file.data == actual);
        if actual != expected && !matches_pending {
            return Err("Project was changed on disk; reopen it before saving".into());
        }
    }
    archive
        .write_files(files.into_iter().map(|file| file.overwrite(true)))
        .map_err(|e| format!("Cannot commit incremental project save: {e}"))
}

pub fn save_archive(path: &Path, files: Vec<WriteFile>) -> Result<(), String> {
    let expected: Vec<_> = files
        .iter()
        .map(|file| (file.path.clone(), fingerprint(&file.data)))
        .collect();
    replace_file(path, |temporary| {
        {
            let archive = MegaFile::create(temporary)
                .map_err(|e| format!("Cannot create temporary project: {e}"))?;
            archive
                .write_files(files)
                .map_err(|e| format!("Cannot write temporary project: {e}"))?;
        }
        let archive = MegaFile::open(temporary)
            .map_err(|e| format!("Cannot verify temporary project: {e}"))?;
        if archive
            .list_all()
            .map_err(|e| format!("Cannot verify project entries: {e}"))?
            .len()
            != expected.len()
        {
            return Err("Temporary project has missing entries".into());
        }
        for (entry, hash) in expected {
            let bytes = archive
                .read_all(&entry)
                .map_err(|e| format!("Cannot verify {entry}: {e}"))?;
            if fingerprint(&bytes) != hash {
                return Err(format!("Temporary project data mismatch: {entry}"));
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn safe_save_replaces_only_with_verified_archive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.mgimg");
        save_archive(
            &path,
            vec![WriteFile::new("/data", "test", b"old".to_vec())],
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        let error = save_archive(
            &path,
            vec![
                WriteFile::new("/duplicate", "test", vec![1]),
                WriteFile::new("/duplicate", "test", vec![2]),
            ],
        );
        assert!(error.is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        save_archive(
            &path,
            vec![WriteFile::new("/data", "test", b"new".to_vec())],
        )
        .unwrap();
        let archive = MegaFile::open(&path).unwrap();
        assert_eq!(archive.read_all("/data").unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn safe_save_partial_write_failure_keeps_original_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.mgimg");
        fs::write(&path, b"previous saved project").unwrap();
        let error = replace_file(&path, |temporary| {
            assert_eq!(temporary.parent(), path.parent());
            fs::write(temporary, b"incomplete data").unwrap();
            Err("Simulated disk write or verification failure".into())
        });
        assert!(error.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"previous saved project");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn safe_save_locked_destination_keeps_previous_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.mgimg");
        fs::write(&path, b"previous saved project").unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(save_archive(&path, vec![WriteFile::new("/data", "test", vec![1])]).is_err());
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), b"previous saved project");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
