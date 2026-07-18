//! Capability-gated I/O for credentials owned by another CLI.
//!
//! Every external open/read stays behind an opaque grant. Consumption opens
//! one absolute regular file through a no-follow traversal, validates that
//! same handle, and reads a bounded payload from it. This prevents a consented
//! path from being redirected through a leaf or parent symlink/reparse point
//! and avoids the old exists-then-read race.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};
use codewhale_config::ExternalCredentialReadGrant;

/// Credential JSON is expected to be tiny. Bound reads so a replaced regular
/// file cannot turn read-only consent into unbounded memory consumption.
const MAX_EXTERNAL_CREDENTIAL_BYTES: u64 = 1024 * 1024;

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(all(test, unix))]
thread_local! {
    static BEFORE_LEAF_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
static OPEN_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static READ_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static WRITE_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static NETWORK_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Open and read the exact granted file once. Missing files are reported as
/// `Ok(None)`; every other unsafe or malformed filesystem shape fails closed.
pub(crate) fn read_to_string(grant: &ExternalCredentialReadGrant) -> Result<Option<String>> {
    #[cfg(test)]
    OPEN_CALLS.fetch_add(1, Ordering::SeqCst);

    let mut file = match open_secure_regular_file(grant.path()) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "securely opening external {} credential file {}",
                    grant.source().as_str(),
                    grant.path().display()
                )
            });
        }
    };

    #[cfg(test)]
    READ_CALLS.fetch_add(1, Ordering::SeqCst);

    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_EXTERNAL_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| {
            format!(
                "reading external {} credential file {}",
                grant.source().as_str(),
                grant.path().display()
            )
        })?;
    if bytes.len() as u64 > MAX_EXTERNAL_CREDENTIAL_BYTES {
        bail!(
            "external {} credential file {} exceeds the {} byte safety limit",
            grant.source().as_str(),
            grant.path().display(),
            MAX_EXTERNAL_CREDENTIAL_BYTES
        );
    }
    let contents = String::from_utf8(bytes).with_context(|| {
        format!(
            "external {} credential file {} is not valid UTF-8",
            grant.source().as_str(),
            grant.path().display()
        )
    })?;
    Ok(Some(contents))
}

#[cfg(unix)]
fn open_secure_regular_file(path: &Path) -> io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external credential path must be absolute",
        ));
    }

    let root = CString::new("/").expect("static root contains no NUL");
    // SAFETY: `root` is a valid C string and flags require no variadic mode.
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `root_fd` is newly owned after the successful `open`.
    let mut current = unsafe { File::from_raw_fd(root_fd) };
    let mut normals = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(Ok(part)),
            Component::RootDir => None,
            Component::Prefix(_) | Component::CurDir | Component::ParentDir => {
                Some(Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "external credential path must be lexically normalized",
                )))
            }
        })
        .peekable();

    let mut opened_leaf = false;
    while let Some(component) = normals.next() {
        let component = component?;
        let component = CString::new(component.as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "external credential path contains a NUL byte",
            )
        })?;
        let leaf = normals.peek().is_none();
        #[cfg(test)]
        if leaf {
            BEFORE_LEAF_OPEN_HOOK.with(|hook| {
                if let Some(hook) = hook.borrow_mut().take() {
                    hook();
                }
            });
        }
        let flags = if leaf {
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK
        } else {
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY
        };
        use std::os::fd::AsRawFd;
        // SAFETY: the directory fd and component C string are valid for this
        // call and flags require no variadic mode.
        let fd = unsafe { libc::openat(current.as_raw_fd(), component.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is newly owned after the successful `openat`.
        current = unsafe { File::from_raw_fd(fd) };
        opened_leaf = leaf;
    }

    if !opened_leaf {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external credential path must name a file",
        ));
    }
    if !current.metadata()?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external credential path must name a regular file",
        ));
    }
    Ok(current)
}

#[cfg(windows)]
fn open_secure_regular_file(path: &Path) -> io::Result<File> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::Component;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
        GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external credential path must be absolute and lexically normalized",
        ));
    }

    // Reject every reparse-point component before the final open. The final
    // handle is opened as the reparse point itself, checked again, and its
    // kernel-resolved path is compared below. A second component pass catches
    // replacement during the open window.
    reject_windows_reparse_components(path)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !metadata.file_type().is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "external credential path must name a non-reparse regular file",
        ));
    }
    reject_windows_reparse_components(path)?;

    let handle = file.as_raw_handle();
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    // SAFETY: the handle remains owned by `file`; null output asks Windows for
    // the required UTF-16 buffer length.
    let needed = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, flags) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; needed as usize + 1];
    // SAFETY: `buffer` is writable for its declared length and `handle` is
    // valid for the duration of the call.
    let written = unsafe {
        GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, flags)
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    let final_path = OsString::from_wide(&buffer[..written as usize]);
    let normalize = |value: &Path| {
        value
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    if normalize(Path::new(&final_path)) != normalize(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "external credential path was redirected while opening",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn reject_windows_reparse_components(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(
            component,
            std::path::Component::Prefix(_) | std::path::Component::RootDir
        ) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&current)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "external credential path contains reparse point {}",
                    current.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn open_secure_regular_file(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure external credential reads are unsupported on this platform",
    ))
}

#[cfg(test)]
pub(crate) fn reset_side_effect_trap() {
    OPEN_CALLS.store(0, Ordering::SeqCst);
    READ_CALLS.store(0, Ordering::SeqCst);
    WRITE_CALLS.store(0, Ordering::SeqCst);
    REFRESH_CALLS.store(0, Ordering::SeqCst);
    NETWORK_CALLS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
#[must_use]
pub(crate) fn side_effect_trap_counts() -> (usize, usize) {
    (
        OPEN_CALLS.load(Ordering::SeqCst),
        READ_CALLS.load(Ordering::SeqCst),
    )
}

#[cfg(test)]
#[must_use]
pub(crate) fn complete_side_effect_trap_counts() -> (usize, usize, usize, usize, usize) {
    (
        OPEN_CALLS.load(Ordering::SeqCst),
        READ_CALLS.load(Ordering::SeqCst),
        WRITE_CALLS.load(Ordering::SeqCst),
        REFRESH_CALLS.load(Ordering::SeqCst),
        NETWORK_CALLS.load(Ordering::SeqCst),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use codewhale_config::{ExternalCredentialConsentToml, ExternalCredentialSource, ProviderKind};

    fn grant(path: &Path) -> ExternalCredentialReadGrant {
        ExternalCredentialConsentToml::read_only(
            ProviderKind::OpenaiCodex,
            ExternalCredentialSource::CodexCli,
            path.to_path_buf(),
        )
        .read_grant(
            ProviderKind::OpenaiCodex,
            ExternalCredentialSource::CodexCli,
            path,
        )
        .expect("test grant")
    }

    #[test]
    fn secure_read_accepts_one_bounded_regular_file() {
        let _env = crate::test_support::lock_test_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .canonicalize()
            .expect("canonical temp root")
            .join("auth.json");
        std::fs::write(&path, "{\"token\":\"ok\"}").expect("fixture");
        assert_eq!(
            read_to_string(&grant(&path))
                .expect("secure read")
                .as_deref(),
            Some("{\"token\":\"ok\"}")
        );
    }

    #[cfg(unix)]
    #[test]
    fn secure_read_rejects_leaf_and_parent_symlinks_and_non_regular_files() {
        let _env = crate::test_support::lock_test_env();
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical temp root");
        let real_dir = root.join("real");
        std::fs::create_dir(&real_dir).expect("real dir");
        let real = real_dir.join("auth.json");
        std::fs::write(&real, "secret").expect("fixture");

        let leaf = root.join("leaf.json");
        symlink(&real, &leaf).expect("leaf symlink");
        assert!(read_to_string(&grant(&leaf)).is_err());

        let parent = root.join("linked-parent");
        symlink(&real_dir, &parent).expect("parent symlink");
        assert!(read_to_string(&grant(&parent.join("auth.json"))).is_err());

        assert!(read_to_string(&grant(&real_dir)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn secure_read_rejects_a_leaf_swapped_after_grant_before_open() {
        let _env = crate::test_support::lock_test_env();
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical temp root");
        let path = root.join("auth.json");
        let moved = root.join("auth-before-swap.json");
        let attacker = root.join("attacker.json");
        std::fs::write(&path, "owner-a").expect("owner fixture");
        std::fs::write(&attacker, "attacker").expect("attacker fixture");
        let grant = grant(&path);
        let hook_path = path.clone();
        BEFORE_LEAF_OPEN_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                std::fs::rename(&hook_path, &moved).expect("move original");
                symlink(&attacker, &hook_path).expect("swap leaf to symlink");
            }));
        });
        assert!(
            read_to_string(&grant).is_err(),
            "a swap to a symlink must fail before any bytes are read"
        );
    }

    #[test]
    fn secure_read_rejects_oversized_regular_file() {
        let _env = crate::test_support::lock_test_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .canonicalize()
            .expect("canonical temp root")
            .join("oversized.json");
        let file = File::create(&path).expect("fixture");
        file.set_len(MAX_EXTERNAL_CREDENTIAL_BYTES + 1)
            .expect("oversize fixture");
        let error = read_to_string(&grant(&path)).expect_err("oversized file");
        assert!(error.to_string().contains("safety limit"), "{error:#}");
    }
}
