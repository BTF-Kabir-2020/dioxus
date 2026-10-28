//! Helpers for writing files that can contain the user's environment.
//!
//! `dx` dumps `std::env::vars()` into the session cache twice: once per crate as
//! `target/dx/.captured-args/<crate>.json` (the rustc wrapper) and once as `link_env.json` (the
//! hotpatch linker interception). Both dumps exist because a faithful link replay needs the full
//! env — rustc hands its linker child vars the rustc process never sees — so dropping keys is not
//! an option. Keeping other local users out of them is: `std::fs::write` respects the umask, which
//! on a default unix setup leaves the dumps at 0644 inside a cache directory that is often
//! traversable.

use std::path::Path;

/// Create `path` (empty) and restrict it to its owner on unix.
///
/// Unlike [`write_private`], this only creates the file and returns the handle, so callers can
/// write to it later while the file is already owner-only from the moment it exists. This closes
/// the window where a freshly created (but not yet written) dump is readable by other users.
pub(crate) fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let file = options.open(path)?;

    // `mode` only applies when the file is created; an existing looser file keeps its old mode.
    restrict_to_owner(path);
    Ok(file)
}

/// Write `contents` to `path`, then restrict the file to its owner on unix.
///
/// The file is created with mode `0600` where the platform supports it, so there is no window in
/// which a freshly written env dump is readable by other users. If the file already exists with
/// looser permissions (both files are pre-created by `prebuild`), its mode is tightened as well.
pub(crate) fn write_private(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;

    let mut file = create_private(path)?;
    file.write_all(contents.as_ref())?;

    // Best-effort tightening again in case the file pre-existed with looser permissions.
    restrict_to_owner(path);
    Ok(())
}

/// Best-effort `chmod 0600`.
///
/// Deliberately infallible: a permission oddity (exotic filesystem, non-owner process) should never
/// break a build, and on platforms without unix modes there is nothing to do.
pub(crate) fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn write_private_creates_owner_only_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env.json");

        write_private(&path, "{\"LIB\":\"C:\\\\sdk\\\\lib\"}").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"LIB\":\"C:\\\\sdk\\\\lib\"}"
        );
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn write_private_tightens_pre_created_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("link_env.json");

        // `prebuild` creates these files empty with the umask's mode before the linker writes them.
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode(&path), 0o644);

        write_private(&path, "[(\"LIB\",\"C:\\\\sdk\\\\lib\")]").unwrap();

        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn create_private_is_owner_only_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env.json");

        let file = create_private(&path).unwrap();

        // The file is already owner-only while it is still empty.
        assert_eq!(mode(&path), 0o600);
        assert!(std::fs::metadata(&path).unwrap().len() == 0);

        drop(file);
    }
}