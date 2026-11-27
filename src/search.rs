use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct App {
    pub name: String,
    pub path: PathBuf,
}

pub fn resolve_pins(apps: &[App], names: &[String]) -> Vec<App> {
    let mut seen = HashSet::new();
    names
        .iter()
        .filter_map(|name| {
            let name = name.trim().to_lowercase();
            if !seen.insert(name.clone()) {
                return None;
            }
            apps.iter()
                .find(|app| app.name.to_lowercase() == name)
                .cloned()
        })
        .collect()
}

pub fn apply_pin_order(pins: &mut [App], names: &[String]) {
    pins.sort_by_cached_key(|pin| {
        names
            .iter()
            .position(|name| *name == pin.name)
            .unwrap_or(usize::MAX)
    });
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

/// Add catalogue entries whose names are not already present, keeping name order.
pub fn merge(apps: &mut Vec<App>, extra: Vec<App>) {
    let mut seen: HashSet<String> = apps.iter().map(|app| app.name.to_lowercase()).collect();
    apps.extend(
        extra
            .into_iter()
            .filter(|app| !app.name.trim().is_empty() && seen.insert(app.name.to_lowercase())),
    );
    apps.sort_by_cached_key(|app| app.name.to_lowercase());
}

/// Every application the Shell lists in its Apps folder, including packaged apps
/// that have no Start menu shortcut file. Each path is a `shell:AppsFolder\<id>`
/// moniker that the Shell can launch.
#[cfg(windows)]
pub fn shell_apps() -> Vec<App> {
    use windows::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{
            BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KNOWN_FOLDER_FLAG,
            SHGetKnownFolderItem, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
        },
    };
    const LIMIT: usize = 4_000;
    // S_OK and S_FALSE need a matching release; RPC_E_CHANGED_MODE does not.
    let initialised = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let name_of = |item: &IShellItem, form| unsafe {
        let text = item.GetDisplayName(form).ok()?;
        let value = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const _));
        value
    };
    let mut apps = Vec::new();
    let listed: windows::core::Result<()> = (|| unsafe {
        let folder: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_AppsFolder, KNOWN_FOLDER_FLAG(0), None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        while apps.len() < LIMIT {
            let mut batch = [None];
            let mut fetched = 0;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            if let (Some(name), Some(id)) = (
                name_of(&item, SIGDN_NORMALDISPLAY),
                name_of(&item, SIGDN_PARENTRELATIVEPARSING),
            ) {
                apps.push(App {
                    name,
                    path: PathBuf::from(format!(r"shell:AppsFolder\{id}")),
                });
            }
        }
        Ok(())
    })();
    drop(listed);
    if initialised {
        unsafe { CoUninitialize() };
    }
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
    fn pins_follow_configuration_order_and_resolve_exact_names() {
        let apps = ["Editor", "Terminal", "Terminal Preview"].map(|name| App {
            name: name.into(),
            path: PathBuf::new(),
        });
        let names = ["terminal", "missing", "Editor", " Terminal "].map(str::to_owned);
        let pins = resolve_pins(&apps, &names);
        assert_eq!(
            pins.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(),
            ["Terminal", "Editor"]
        );
    }

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
    fn saved_order_reorders_current_pins_and_leaves_new_pins_at_the_end() {
        let mut pins = ["Editor", "Terminal", "Browser"].map(|name| App {
            name: name.into(),
            path: PathBuf::new(),
        });
        apply_pin_order(
            &mut pins,
            &["Terminal".into(), "Old".into(), "Editor".into()],
        );
        assert_eq!(pins.map(|pin| pin.name), ["Terminal", "Editor", "Browser"]);
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

    #[test]
    fn merges_only_names_not_already_catalogued() {
        let app = |name: &str, path: &str| App {
            name: name.into(),
            path: path.into(),
        };
        let mut apps = vec![
            app("Visual Studio Code", "code.lnk"),
            app("Notepad", "notepad.lnk"),
        ];
        merge(
            &mut apps,
            vec![
                app("notepad", r"shell:AppsFolder\Notepad!App"),
                app(
                    "Terminal",
                    r"shell:AppsFolder\Microsoft.WindowsTerminal!App",
                ),
                app(" ", r"shell:AppsFolder\Blank"),
            ],
        );
        let names: Vec<_> = apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["Notepad", "Terminal", "Visual Studio Code"]);
        assert_eq!(apps[0].path, PathBuf::from("notepad.lnk"));
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "reads the interactive user's Apps folder"]
    fn lists_packaged_applications() {
        let apps = shell_apps();
        assert!(
            apps.iter()
                .any(|app| app.path.to_string_lossy().contains('!')),
            "no packaged application among {} entries",
            apps.len()
        );
    }
}
