//! Path helpers that reproduce Python `pathlib` behaviour the 1.x CLI exposed
//! in its output (`str(Path(...))`, `Path.resolve()`, `Path.name`).

use std::path::{Component, Path, PathBuf};

/// `str(pathlib.PurePosixPath(raw))`: collapse repeated separators and `.`
/// components and drop a trailing separator; `..` is kept as written.
pub fn display(raw: &str) -> String {
    if raw.is_empty() {
        return ".".to_string();
    }
    let path = Path::new(raw);
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    let text = out.to_string_lossy().into_owned();
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

/// `Path.name`: the final component, or "" for a root.
pub fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `Path(p).resolve()` (non-strict): make `p` absolute against the current
/// directory, resolve symlinks for the part that exists, and apply the rest of
/// the components lexically.
pub fn resolve(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(p)
    };
    let mut resolved = PathBuf::new();
    let mut existing = true;
    for comp in abs.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => resolved.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => {
                resolved.push(part);
                if existing {
                    match resolved.canonicalize() {
                        Ok(real) => resolved = strip_verbatim(real),
                        Err(_) => existing = false,
                    }
                }
            }
        }
    }
    resolved
}

/// `Path.resolve()` for a path that exists; falls back to [`resolve`].
pub fn canonical(p: &Path) -> PathBuf {
    p.canonicalize()
        .map(strip_verbatim)
        .unwrap_or_else(|_| resolve(p))
}

/// Windows `canonicalize` returns `\\?\C:\...`; Python prints `C:\...`.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    if cfg!(windows) {
        let text = p.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if !rest.starts_with("UNC\\") {
                return PathBuf::from(rest);
            }
        }
    }
    p
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn display_normalises_like_pathlib() {
        assert_eq!(display("/a//b/./c/"), "/a/b/c");
        assert_eq!(display("/a/../b"), "/a/../b");
        assert_eq!(display(""), ".");
    }

    #[test]
    fn resolve_handles_missing_tail() {
        let base = std::env::temp_dir().canonicalize().unwrap();
        let got = resolve(&base.join("does-not-exist/x/../y"));
        assert_eq!(got, base.join("does-not-exist/y"));
    }
}
