//! UID → package names, with the same two-level caching as `mem apps`.
//!
//! `/data/system/packages.list` is a plain file and is the primary source; `cmd package list
//! packages -U` is only a fallback. The map changes on install/uninstall, so it is cached for ten
//! minutes under `<data_dir>/state/process_packages.json` — the process list itself is never cached.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use sysflow_core::batterystats::read_package_uids;
use sysflow_core::cache;
use sysflow_core::sysfs::Sysfs;

const PACKAGES_LIST: &str = "/data/system/packages.list";
const PACKAGES_TTL_SECS: u64 = 10 * 60;

#[derive(Clone, Debug, Default)]
pub struct Packages {
    /// Keyed by app id (`uid % 100000`), exactly like the `packages.list` first column.
    by_app_id: BTreeMap<u32, Vec<String>>,
}

impl Packages {
    /// All package names sharing this UID (shared UIDs are rare but real).
    pub fn names(&self, uid: u32) -> Option<&[String]> {
        self.by_app_id.get(&(uid % 100_000)).map(Vec::as_slice)
    }

    /// The name a UI should show, if this UID owns one.
    pub fn primary(&self, uid: u32) -> Option<&str> {
        self.names(uid)
            .and_then(|names| names.first())
            .map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.by_app_id.is_empty()
    }
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state").join("process_packages.json")
}

pub fn read(fs: &Sysfs, data_dir: &Path) -> Packages {
    let path = cache_path(data_dir);
    if let Some(cached) = cache::read_fresh(&path, PACKAGES_TTL_SECS) {
        if let Some(map) = parse_cached(&cached) {
            return Packages { by_app_id: map };
        }
    }

    let fresh = match fs.read_lossy(PACKAGES_LIST) {
        Ok(text) => {
            let map = parse_packages_list(&text);
            if map.is_empty() {
                read_package_uids()
            } else {
                map
            }
        }
        Err(_) => read_package_uids(),
    };

    if !fresh.is_empty() {
        let packages: Map<String, Value> = fresh
            .iter()
            .map(|(uid, names)| (uid.to_string(), json!(names)))
            .collect();
        cache::write(
            &path,
            &json!({
                "generated_at": cache::now_secs(),
                "packages": Value::Object(packages),
            }),
        );
    }
    Packages { by_app_id: fresh }
}

/// `/data/system/packages.list`: one line per package, whitespace separated, the first field is the
/// package name and the second is its app id. The rest of the line is metadata we do not need.
fn parse_packages_list(text: &str) -> BTreeMap<u32, Vec<String>> {
    let mut out: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(package), Some(uid)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else {
            continue;
        };
        out.entry(uid).or_default().push(package.to_string());
    }
    out
}

fn parse_cached(value: &Value) -> Option<BTreeMap<u32, Vec<String>>> {
    let object = value.get("packages")?.as_object()?;
    let mut map = BTreeMap::new();
    for (key, names) in object {
        let uid = key.parse::<u32>().ok()?;
        let list: Vec<String> = names
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        map.insert(uid, list);
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_map_round_trips() {
        let value = json!({
            "generated_at": 1,
            "packages": {"10157": ["com.android.systemui"], "10346": ["com.example.app", "com.example.app2"]}
        });
        let map = parse_cached(&value).expect("cache parses");
        let packages = Packages { by_app_id: map };
        assert_eq!(packages.primary(10157), Some("com.android.systemui"));
        assert_eq!(
            packages.names(10_10346).map(<[String]>::to_vec),
            Some(vec![
                "com.example.app".to_string(),
                "com.example.app2".to_string()
            ]),
            "work-profile UIDs match on uid % 100000"
        );
        assert_eq!(packages.primary(99999), None);
    }

    #[test]
    fn packages_list_parses_name_and_app_id() {
        let text = "com.example.app 10123 0 /data/user/0/com.example.app default:targetSdkVersion=34 0 1 0 0 0\ncom.example.other 10456 0 /data 0 0 1 0 0 0\nbroken line\n";
        let map = parse_packages_list(text);
        assert_eq!(
            map.get(&10123).map(Vec::as_slice),
            Some(&["com.example.app".to_string()][..])
        );
        assert_eq!(
            map.get(&10456).map(Vec::as_slice),
            Some(&["com.example.other".to_string()][..])
        );
        assert_eq!(
            map.len(),
            2,
            "a line without a numeric uid is ignored, not an error"
        );
    }

    #[test]
    fn malformed_cache_is_ignored_instead_of_panicking() {
        assert!(parse_cached(&json!({"packages": {"not-auth": ["x"]}})).is_none());
        assert!(parse_cached(&json!({})).is_none());
        assert!(parse_cached(&json!([])).is_none());
    }
}
