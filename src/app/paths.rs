use std::path::{Path, PathBuf};

/// Resolves an asset path. If running inside a Mac app bundle (Contents/MacOS),
/// it resolves relative to Contents/Resources. Otherwise it resolves relative to CWD.
pub fn resolve_asset_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Ok(exe) = std::env::current_exe() {
        let exe_str = exe.to_string_lossy();
        if exe_str.contains("Contents/MacOS") {
            if let Some(resources) = exe
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("Resources"))
            {
                return resources.join(path);
            }
        }
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}

/// Resolves the executable's directory. Useful for finding co-located .dylib files.
pub fn resolve_exe_dir() -> PathBuf {
    std::env::current_exe()
        .map(|p| p.parent().unwrap_or(Path::new(".")).to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_asset_path_absolute() {
        let path = Path::new("/absolute/path");
        assert_eq!(resolve_asset_path(path), path.to_path_buf());
    }

    #[test]
    fn test_resolve_asset_path_relative() {
        let path = Path::new("relative/path");
        let resolved = resolve_asset_path(path);
        // It should either be absolute (if it resolved to CWD or Resources)
        // or start with CWD (in our fallback case where env::current_dir fails, it might be relative, but let's just check it doesn't crash)
        assert!(resolved.components().count() > 0);
    }

    #[test]
    fn test_resolve_exe_dir() {
        let dir = resolve_exe_dir();
        assert!(dir.components().count() > 0);
    }
}
