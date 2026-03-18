use ignore::WalkBuilder;
use std::path::Path;

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
