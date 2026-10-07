//! User PATH in the Windows registry (`HKCU\Environment`).
//!
//! Ported from gvsn `commands/setup.rs`. Makes nvsn's global version visible
//! to all applications (editors, start menus, processes launched by
//! Explorer), not only to interactive shells (ADR-034).
//!
//! Rules:
//! - Only the entries nvsn adds are touched. The others are kept as they are.
//! - The comparison ignores case and the trailing slash (Windows does not distinguish them).
//! - The value is written with the same type it had (`REG_EXPAND_SZ` if it was
//!   new), so the `%VAR%` references of other entries are not broken.
//! - The registry is written only if something changes, and only then is the
//!   `WM_SETTINGCHANGE` broadcast sent.
//!
//! The merge and removal functions are pure and are tested on any OS.
//! Registry I/O is Windows-only.

/// User PATH key, relative to `HKEY_CURRENT_USER`.
pub const ENVIRONMENT_KEY: &str = "Environment";

/// Result of editing a list of `;`-separated entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Edit {
    /// New PATH value.
    pub value: String,
    /// Entries that were added or removed. Empty means no change.
    pub touched: Vec<String>,
}

/// Normalizes an entry for comparison: no trailing slash and lowercase.
#[cfg_attr(not(windows), allow(dead_code))]
fn normalize(entry: &str) -> String {
    entry
        .trim()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

/// Windows path as text with `\`, so it matches what the installer writes
/// and what the user sees.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn backslash_entry(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

/// Adds the entries of `add` that are not yet in the PATH. New ones are prepended,
/// as gvsn does, so they take precedence over other installed versions.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn merge_entries(current: &str, add: &[String]) -> Edit {
    let mut entries: Vec<String> = split(current);
    let mut touched = Vec::new();
    for candidate in add {
        let wanted = normalize(candidate);
        if wanted.is_empty() {
            continue;
        }
        if entries.iter().any(|e| normalize(e) == wanted) {
            continue;
        }
        entries.insert(0, candidate.clone());
        touched.push(candidate.clone());
    }
    Edit {
        value: entries.join(";"),
        touched,
    }
}

/// Removes from the PATH the entries equal to a path under `under` or located
/// inside it. Only paths under nvsn's directory are removed; the rest is unchanged.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn remove_under(current: &str, under: &str) -> Edit {
    let root = normalize(under);
    let prefix = format!("{root}\\");
    let mut touched = Vec::new();
    let kept: Vec<String> = split(current)
        .into_iter()
        .filter(|entry| {
            let norm = normalize(entry);
            let matches = norm == root || norm.starts_with(&prefix);
            if matches {
                touched.push(entry.clone());
            }
            !matches
        })
        .collect();
    Edit {
        value: kept.join(";"),
        touched,
    }
}

/// Splits the PATH into entries, discarding the empty ones.
#[cfg_attr(not(windows), allow(dead_code))]
fn split(current: &str) -> Vec<String> {
    current
        .split(';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Adds `entries` to the user PATH at `HKCU\<subkey>`. Returns those that were
/// missing and have been added. If all of them were already present, nothing is written.
///
/// # Errors
///
/// Fails if the key cannot be opened, if the stored PATH is not valid UTF-16,
/// or if it cannot be written.
#[cfg(windows)]
pub fn add_user_path_entries(
    subkey: &str,
    entries: &[std::path::PathBuf],
) -> anyhow::Result<Vec<String>> {
    let wanted: Vec<String> = entries.iter().map(|p| backslash_entry(p)).collect();
    let edit = edit_user_path(subkey, |current| merge_entries(current, &wanted))?;
    Ok(edit.touched)
}

/// Removes from the user PATH at `HKCU\<subkey>` the entries under `root`. Returns
/// those that were removed. Nothing is written if there were none.
///
/// # Errors
///
/// Same cases as [`add_user_path_entries`].
#[cfg(windows)]
pub fn remove_user_path_under(subkey: &str, root: &std::path::Path) -> anyhow::Result<Vec<String>> {
    let root = backslash_entry(root);
    let edit = edit_user_path(subkey, |current| remove_under(current, &root))?;
    Ok(edit.touched)
}

/// Reads the PATH, applies `change`, and writes only if there were changes.
#[cfg(windows)]
fn edit_user_path(subkey: &str, change: impl FnOnce(&str) -> Edit) -> anyhow::Result<Edit> {
    use anyhow::Context;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ};
    use winreg::{RegKey, RegValue};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(subkey, KEY_READ | KEY_WRITE)
        .with_context(|| format!("Cannot open HKCU\\{subkey}"))?;

    let (current, vtype) = match key.get_raw_value("PATH") {
        Ok(raw) => (decode_utf16(&raw.bytes)?, raw.vtype),
        Err(_) => (String::new(), REG_EXPAND_SZ),
    };

    let edit = change(&current);
    if edit.touched.is_empty() {
        return Ok(edit);
    }

    let raw = RegValue {
        bytes: encode_utf16(&edit.value),
        vtype,
    };
    key.set_raw_value("PATH", &raw)
        .with_context(|| format!("Cannot write PATH to HKCU\\{subkey}"))?;

    if subkey == ENVIRONMENT_KEY {
        broadcast_environment_change();
    }
    Ok(edit)
}

/// Decodes NUL-terminated UTF-16LE, as the registry stores it.
#[cfg(windows)]
fn decode_utf16(bytes: &[u8]) -> anyhow::Result<String> {
    use anyhow::Context;
    let (pairs, _odd_byte) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16(&units).context("The user PATH is not valid UTF-16; it is left unchanged")
}

/// Encodes `value` as UTF-16LE with the trailing NUL the registry expects.
#[cfg(windows)]
fn encode_utf16(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Notifies Explorer and the processes that inherit from it that the environment changed.
///
/// Without this, a graphical application launched right afterwards still sees the
/// old PATH (ADR-034). It is best-effort: if it fails, the result is the same as
/// without the notice (the session must be restarted).
///
/// This is nvsn's only `unsafe` call: the Win32 API has no safe equivalent.
/// See the exception documented in ADR-034.
#[cfg(windows)]
#[allow(unsafe_code)]
fn broadcast_environment_change() {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let param: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result: usize = 0;
    // SAFETY: `param` is a NUL-terminated UTF-16 buffer that stays alive for the
    // whole call, and `result` is a valid `usize` where Win32 writes the
    // result. SMTO_ABORTIFHUNG with a 5 s timeout bounds the wait.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as HWND,
            WM_SETTINGCHANGE,
            0,
            param.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn merge_prepends_missing_entries_and_keeps_the_rest() {
        let edit = merge_entries(r"C:\Windows;C:\tools", &strings(&[r"C:\nvsn\current"]));
        assert_eq!(edit.value, r"C:\nvsn\current;C:\Windows;C:\tools");
        assert_eq!(edit.touched, strings(&[r"C:\nvsn\current"]));
    }

    #[test]
    fn merge_is_idempotent_and_case_insensitive() {
        let current = r"c:\users\jhon\nvsn\current;C:\Windows";
        let edit = merge_entries(current, &strings(&[r"C:\Users\Jhon\nvsn\current\"]));
        assert!(edit.touched.is_empty());
        assert_eq!(edit.value, current);
    }

    #[test]
    fn merge_does_not_duplicate_within_the_same_call() {
        let edit = merge_entries("", &strings(&[r"C:\a", r"c:\a"]));
        assert_eq!(edit.touched, strings(&[r"C:\a"]));
        assert_eq!(edit.value, r"C:\a");
    }

    #[test]
    fn merge_skips_empty_candidates_and_empty_current() {
        let edit = merge_entries(";;", &strings(&["", r"C:\x"]));
        assert_eq!(edit.value, r"C:\x");
        assert_eq!(edit.touched, strings(&[r"C:\x"]));
    }

    #[test]
    fn remove_drops_root_and_children_only() {
        let current = r"C:\nvsn\current;C:\other;C:\nvsn-other\bin;C:\NVSN\versions\x";
        let edit = remove_under(current, r"C:\nvsn");
        assert_eq!(edit.value, r"C:\other;C:\nvsn-other\bin");
        assert_eq!(
            edit.touched,
            strings(&[r"C:\nvsn\current", r"C:\NVSN\versions\x"])
        );
    }

    #[test]
    fn remove_without_matches_reports_no_change() {
        let edit = remove_under(r"C:\other", r"C:\nvsn");
        assert!(edit.touched.is_empty());
        assert_eq!(edit.value, r"C:\other");
    }

    #[test]
    fn backslash_entry_converts_forward_slashes() {
        let path = std::path::Path::new("C:/Users/x/nvsn/current");
        assert_eq!(backslash_entry(path), r"C:\Users\x\nvsn\current");
    }

    /// Test key under `HKCU\Software`, deleted when the scope exits. `HKCU\Environment`
    /// is never used in the tests.
    #[cfg(windows)]
    struct TestKey {
        subkey: String,
    }

    #[cfg(windows)]
    impl TestKey {
        fn new(name: &str, initial_path: Option<&str>) -> Self {
            use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, REG_EXPAND_SZ};
            use winreg::{RegKey, RegValue};

            let subkey = format!("Software\\nvsn-test-{}-{name}", std::process::id());
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let (key, _) = hkcu
                .create_subkey_with_flags(&subkey, KEY_ALL_ACCESS)
                .expect("create test key");
            if let Some(value) = initial_path {
                let raw = RegValue {
                    bytes: encode_utf16(value),
                    vtype: REG_EXPAND_SZ,
                };
                key.set_raw_value("PATH", &raw).expect("seed PATH");
            }
            Self { subkey }
        }

        fn read_path(&self) -> (String, winreg::enums::RegType) {
            use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
            use winreg::RegKey;

            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(&self.subkey, KEY_READ)
                .expect("open test key");
            let raw = key.get_raw_value("PATH").expect("read PATH");
            (decode_utf16(&raw.bytes).expect("UTF-16"), raw.vtype)
        }
    }

    #[cfg(windows)]
    impl Drop for TestKey {
        fn drop(&mut self) {
            use winreg::enums::HKEY_CURRENT_USER;
            use winreg::RegKey;
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.subkey);
        }
    }

    /// Writes to the test key using the production functions, with the same logic
    /// as `add_user_path_entries` but pointing at `subkey`.
    #[cfg(windows)]
    fn add_to(subkey: &str, entries: &[&std::path::Path]) -> Vec<String> {
        // `add_user_path_entries` only accepts the real key; the tests use
        // the test key through `edit_user_path` directly.
        let wanted: Vec<String> = entries.iter().map(|p| backslash_entry(p)).collect();
        edit_user_path(subkey, |current| merge_entries(current, &wanted))
            .expect("edit")
            .touched
    }

    #[cfg(windows)]
    #[test]
    fn registry_add_keeps_other_entries_and_is_idempotent() {
        let key = TestKey::new("add", Some(r"C:\Windows;%USERPROFILE%\bin"));
        let nvsn_dir = std::path::Path::new(r"C:\Users\x\AppData\Local\nvsn");
        let current = nvsn_dir.join("current");

        let added = add_to(&key.subkey, &[&current]);
        assert_eq!(added, vec![backslash_entry(&current)]);

        let (value, vtype) = key.read_path();
        assert_eq!(
            value,
            format!(
                r"{};C:\Windows;%USERPROFILE%\bin",
                backslash_entry(&current)
            )
        );
        assert_eq!(vtype, winreg::enums::REG_EXPAND_SZ, "keeps the type");

        assert!(
            add_to(&key.subkey, &[&current]).is_empty(),
            "does not duplicate"
        );
        assert_eq!(key.read_path().0.matches("nvsn").count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn registry_creates_path_when_missing() {
        let key = TestKey::new("create", None);
        let dir = std::path::Path::new(r"C:\nvsn");
        assert_eq!(add_to(&key.subkey, &[dir]), vec![r"C:\nvsn".to_owned()]);
        assert_eq!(key.read_path().0, r"C:\nvsn");
    }

    #[cfg(windows)]
    #[test]
    fn registry_remove_only_drops_nvsn_entries() {
        let key = TestKey::new("remove", Some(r"C:\nvsn\current;C:\tools;C:\nvsn-other"));
        let edit =
            edit_user_path(&key.subkey, |current| remove_under(current, r"C:\nvsn")).expect("edit");
        assert_eq!(edit.touched, vec![r"C:\nvsn\current".to_owned()]);
        assert_eq!(key.read_path().0, r"C:\tools;C:\nvsn-other");
    }

    #[cfg(windows)]
    #[test]
    fn registry_does_not_write_when_nothing_changes() {
        let key = TestKey::new("nochange", Some(r"C:\tools"));
        let edit =
            edit_user_path(&key.subkey, |current| remove_under(current, r"C:\nvsn")).expect("edit");
        assert!(edit.touched.is_empty());
        assert_eq!(key.read_path().0, r"C:\tools");
    }
}
