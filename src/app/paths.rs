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
            if let Some(resources) = exe.parent().and_then(|p| p.parent()).map(|p| p.join("Resources")) {
                return resources.join(path);
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
}

/// Resolves the executable's directory. Useful for finding co-located .dylib files.
pub fn resolve_exe_dir() -> PathBuf {
    std::env::current_exe()
        .map(|p| p.parent().unwrap_or(Path::new(".")).to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("."))
}
