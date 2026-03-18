//! Import path resolution utilities.

use std::path::{Path, PathBuf};

/// Common source file extensions for import resolution.
/// Ordered by likelihood for fast resolution.
const SOURCE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mts", "mjs", "cjs",  // JavaScript/TypeScript
    "rs",                                              // Rust
    "py", "pyi",                                       // Python
    "go",                                              // Go
    "rb",                                              // Ruby
    "java", "kt", "scala",                            // JVM
    "cs",                                              // C#
    "swift",                                           // Swift
    "c", "h", "cpp", "hpp", "cc",                     // C/C++
];

/// Resolve a relative import path to an actual file.
pub fn resolve_import(import_path: &str, from_file: &Path) -> Option<PathBuf> {
    let dir = from_file.parent()?;
    resolve_import_in(import_path, dir)
}

/// Resolve a relative import path within a directory.
pub fn resolve_import_in(import_path: &str, dir: &Path) -> Option<PathBuf> {
    let candidate = dir.join(import_path);

    // Try exact path
    if candidate.exists() && candidate.is_file() {
        return Some(candidate);
    }

    // Try common extensions
    for ext in SOURCE_EXTENSIONS {
        let with_ext = candidate.with_extension(ext);
        if with_ext.exists() {
            return Some(with_ext);
        }
    }

    // Try index file in directory
    for ext in SOURCE_EXTENSIONS {
        let index = candidate.join(format!("index.{}", ext));
        if index.exists() {
            return Some(index);
        }
    }

    None
}
