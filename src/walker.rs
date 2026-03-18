use ignore::WalkBuilder;
use std::path::Path;
use std::ffi::OsStr;

/// Standard noise directories excluded from all walks.
pub const NOISE_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".yarn",
    "dist",
    "build",
    ".next",
    "__pycache__",
    "target",
    ".turbo",
    ".cache",
    ".parcel-cache",
];

/// Build a gitignore-aware walker with standard noise filtering.
pub fn build_walker(path: &Path, git_ignore: bool) -> WalkBuilder {
    let mut builder = WalkBuilder::new(path);
    builder
        .git_ignore(git_ignore)
        .git_global(git_ignore)
        .hidden(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !NOISE_DIRS.contains(&name.as_ref())
        });
    builder
}

/// Check if a file path looks like a test file.
pub fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new(""))
        .to_string_lossy();
    name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.starts_with("test_")
        || name.contains(".mocha.")
}
