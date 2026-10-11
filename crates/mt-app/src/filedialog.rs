//! Native file dialogs (through `rfd`), with a test hook: when the environment
//! variable `MESHTARD_TEST_DIALOG` names a file, every dialog answers with the path in
//! that file's first line instead (empty: cancelled). Used for GUI checks where no
//! desktop portal is running.

use std::path::PathBuf;

fn test_answer() -> Option<Option<PathBuf>> {
    let hook = std::env::var_os("MESHTARD_TEST_DIALOG")?;
    let text = std::fs::read_to_string(hook).unwrap_or_default();
    let line = text.lines().next().unwrap_or("").trim();
    Some((!line.is_empty()).then(|| PathBuf::from(line)))
}

/// A "Save" dialog. `filters` are (name, extensions); a chosen name without an
/// extension gets the first filter's first extension.
pub(crate) fn save(filters: &[(&str, &[&str])], file_name: &str) -> Option<PathBuf> {
    let path = match test_answer() {
        Some(answer) => answer,
        None => {
            let mut d = rfd::FileDialog::new().set_file_name(file_name);
            for (name, exts) in filters {
                d = d.add_filter(*name, exts);
            }
            d.save_file()
        }
    }?;
    Some(match (path.extension(), filters.first()) {
        (None, Some((_, [ext, ..]))) => path.with_extension(ext),
        _ => path,
    })
}

/// An "Open" dialog.
pub(crate) fn open(filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    if let Some(answer) = test_answer() {
        return answer;
    }
    let mut d = rfd::FileDialog::new();
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d.pick_file()
}
