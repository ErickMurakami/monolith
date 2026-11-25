use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub struct App {
    pub name: String,
    pub path: PathBuf,
}

fn scan(dir: &Path, depth: usize, apps: &mut Vec<App>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            scan(&path, depth - 1, apps);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
            && let Some(name) = path.file_stem()
        {
            apps.push(App {
                name: name.to_string_lossy().into_owned(),
                path: path.clone(),
            });
        }
    }
}

pub fn discover(roots: impl IntoIterator<Item = PathBuf>) -> Vec<App> {
    let mut apps = Vec::new();
    for root in roots {
        scan(&root, 12, &mut apps);
    }
    let mut seen = HashSet::new();
    apps.retain(|app| seen.insert(app.name.to_lowercase()));
    apps.sort_by_cached_key(|app| app.name.to_lowercase());
    apps
}

fn rank(name: &str, query: &str) -> Option<u8> {
    if name.starts_with(query) {
        return Some(0);
    }
    if name.contains(query) {
        return Some(1);
    }
    let mut chars = name.chars();
    query
        .chars()
        .all(|needle| chars.any(|ch| ch == needle))
        .then_some(2)
}

pub fn search<'a>(
    titles: impl IntoIterator<Item = &'a str>,
    query: &str,
    limit: usize,
) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut hits: Vec<_> = titles
        .into_iter()
        .enumerate()
        .filter_map(|(index, title)| rank(&title.to_lowercase(), &query).map(|rank| (rank, index)))
        .collect();
    hits.sort_unstable();
    hits.into_iter()
        .take(limit)
        .map(|(_, index)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_prefix_then_substring_then_subsequence() {
        let titles = [
            "Visual Studio Code",
            "My Code Editor",
            "Code Tools",
            "Terminal",
        ];
        assert_eq!(search(titles, " CoDE ", 3), [2, 0, 1]);
        assert_eq!(search(titles, "vsc", 1), [0]);
        assert!(search(titles, "zz", 10).is_empty());
    }

    #[test]
    fn discovers_only_shortcuts_and_prefers_user_entries() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("monolith-search-{}", std::process::id()));
        let user = root.join("user");
        let shared = root.join("shared");
        fs::create_dir_all(user.join("nested"))?;
        fs::create_dir_all(&shared)?;
        let expected = user.join("nested").join("Editor.LNK");
        fs::write(&expected, [])?;
        fs::write(shared.join("editor.lnk"), [])?;
        fs::write(shared.join("other.exe"), [])?;
        let apps = discover([user, shared]);
        fs::remove_dir_all(root)?;
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].path, expected);
        Ok(())
    }
}
