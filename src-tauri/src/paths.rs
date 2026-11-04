//! Filesystem confinement for the application-owned games root.
//! All copy and launch targets are validated to stay under a single root directory.
use std::path::{Path, PathBuf};

/// Reusable error type for path validation failures.
#[derive(Debug, PartialEq)]
pub enum PathError {
    InvalidComponent,
    Traversal,
    AbsolutePath,
    RootOrPrefix,
    DevicePath,
    EmptyName,
    InvalidFilename,
    NotUnderRoot,
    SymlinkAncestor,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            PathError::InvalidComponent => "path contains an invalid component",
            PathError::Traversal => "path contains parent-directory traversal",
            PathError::AbsolutePath => "absolute paths are not allowed",
            PathError::RootOrPrefix => "root or prefix components are not allowed",
            PathError::DevicePath => "device paths are not allowed",
            PathError::EmptyName => "name must not be empty",
            PathError::InvalidFilename => "filename contains invalid characters",
            PathError::NotUnderRoot => "resolved path escapes the games root",
            PathError::SymlinkAncestor => "path contains a symlink or junction ancestor",
        };
        write!(f, "{}", msg)
    }
}

impl std::error::Error for PathError {}

/// Returns the application-owned games root: `<exe_dir>/games`.
pub fn games_root() -> Result<PathBuf, std::io::Error> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path.parent().unwrap_or_else(|| Path::new(""));
    Ok(exe_dir.join("games"))
}

/// Validate a single path component (directory name). Rejects `.`, `..`, empty,
/// components containing separators, and Windows reserved device names.
pub fn validate_component(name: &str) -> Result<(), PathError> {
    if name.is_empty() {
        return Err(PathError::EmptyName);
    }
    if name == "." || name == ".." {
        return Err(PathError::InvalidComponent);
    }
    if name.chars().any(|c| c.is_control() || "/\\:<>\"|?*".contains(c)) {
        return Err(PathError::InvalidComponent);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(PathError::InvalidComponent);
    }
    let upper = name.to_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&base) {
        return Err(PathError::InvalidFilename);
    }
    Ok(())
}

/// Validate an executable filename. Must be a single component.
pub fn validate_executable_name(name: &str) -> Result<(), PathError> {
    if name.is_empty() {
        return Err(PathError::EmptyName);
    }
    validate_component(name)?;
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(PathError::InvalidFilename);
    }
    Ok(())
}

/// Build a confined path under the games root for the given app_id, relative
/// path string, and executable name. Resolves existing ancestors and verifies
/// the result stays under the root and does not traverse symlink/junction ancestors.
pub fn resolve_confined_path(
    root: &Path,
    app_id: i64,
    path: &str,
    executable_name: &str,
) -> Result<PathBuf, PathError> {
    validate_executable_name(executable_name)?;

    let mut components: Vec<&str> = Vec::new();
    if !path.is_empty() && path != "." {
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(PathError::AbsolutePath);
        }
        if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
            return Err(PathError::RootOrPrefix);
        }
        if path.len() >= 3 {
            let bytes = path.as_bytes();
            if bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
                return Err(PathError::AbsolutePath);
            }
        }
        for component in path.split(|c| c == '/' || c == '\\') {
            if component.is_empty() {
                continue;
            }
            if component == ".." {
                return Err(PathError::Traversal);
            }
            validate_component(component)?;
            components.push(component);
        }
    }

    if !root.is_absolute() || app_id <= 0 {
        return Err(PathError::NotUnderRoot);
    }
    // Validate root and app-id directory before canonicalization: neither may
    // be a junction whose outside destination would otherwise become trusted.
    for ancestor in root.ancestors() {
        reject_link(ancestor)?;
    }
    let canonical_root = if root.exists() {
        std::fs::canonicalize(root).map_err(|_| PathError::NotUnderRoot)?
    } else {
        root.to_path_buf()
    };
    let mut current = canonical_root.join(app_id.to_string());
    reject_link(&current)?;

    for component in components.iter().chain(std::iter::once(&executable_name)) {
        let next = current.join(component);
        reject_link(&next)?;
        if next.exists() {
            current = std::fs::canonicalize(&next).map_err(|_| PathError::NotUnderRoot)?;
        } else {
            current = next;
        }
        if !current.starts_with(&canonical_root) {
            return Err(PathError::NotUnderRoot);
        }
    }

    Ok(current)
}

fn reject_link(path: &Path) -> Result<(), PathError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if is_symlink_or_junction(&metadata) => Err(PathError::SymlinkAncestor),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PathError::NotUnderRoot),
    }
}

pub fn verify_runner(target: &Path, template: &Path) -> Result<(), String> {
    let expected = std::fs::read(template).map_err(|e| format!("cannot read packaged runner: {e}"))?;
    let actual = std::fs::read(target).map_err(|e| format!("cannot read installed runner: {e}"))?;
    if expected.is_empty() || actual != expected {
        return Err("file does not match the packaged runner".into());
    }
    Ok(())
}

#[cfg(windows)]
fn is_symlink_or_junction(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn is_symlink_or_junction(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("dqc-path-test-{}-{}", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rejects_parent_traversal() {
        let root = temp_root();
        let result = resolve_confined_path(&root, 123, "../escape", "game.exe");
        assert!(matches!(
            result,
            Err(PathError::Traversal | PathError::NotUnderRoot)
        ));
    }

    #[test]
    fn rejects_absolute_path() {
        let root = temp_root();
        let result = resolve_confined_path(&root, 123, r"C:\Windows\evil", "game.exe");
        assert!(matches!(result, Err(PathError::AbsolutePath)));
    }

    #[test]
    fn rejects_empty_and_relative_filenames() {
        let root = temp_root();
        assert!(resolve_confined_path(&root, 123, "foo", "").is_err());
        assert!(resolve_confined_path(&root, 123, "foo", ".").is_err());
        assert!(resolve_confined_path(&root, 123, "foo", "..").is_err());
    }

    #[test]
    fn rejects_device_names() {
        let root = temp_root();
        assert!(resolve_confined_path(&root, 123, "foo", "CON.exe").is_err());
        assert!(resolve_confined_path(&root, 123, "foo", "COM1").is_err());
    }

    #[test]
    fn resolves_simple_path_under_root() {
        let root = temp_root();
        let path = resolve_confined_path(&root, 123, "foo/bar", "game.exe").unwrap();
        assert!(path.starts_with(fs::canonicalize(&root).unwrap()));
        assert!(path.to_string_lossy().contains("123"));
        assert!(path.to_string_lossy().contains("foo"));
        assert!(path.to_string_lossy().contains("bar"));
        assert!(path.ends_with("game.exe"));
    }

    #[test]
    fn detects_symlink_escape() {
        let root = temp_root().join("games");
        let app_root = root.join("123");
        fs::create_dir_all(&app_root).unwrap();
        let outside = root.parent().unwrap().join("outside");
        fs::create_dir_all(&outside).unwrap();

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let status = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"]).arg(app_root.join("escape")).arg(&outside)
                .creation_flags(0x08000000).status().unwrap();
            assert!(status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, app_root.join("escape")).unwrap();

        let result = resolve_confined_path(&root, 123, "escape", "game.exe");
        assert!(matches!(
            result,
            Err(PathError::SymlinkAncestor | PathError::NotUnderRoot)
        ));
    }
}
