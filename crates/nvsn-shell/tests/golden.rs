//! Snapshot (golden) tests of the scripts generated per shell.
//!
//! Each shell generates `env_script` and `wrapper_function` from the same input: a
//! path with a space and an accent. The results are compared with the files in
//! `tests/golden/`. To regenerate them after an intended change:
//!
//! ```text
//! NVSN_BLESS=1 cargo test -p nvsn-shell --test golden
//! ```
//!
//! The comparison normalizes line endings, in case git changes CRLF and LF.

use nvsn_shell::{from_str, EnvContext, ShellConfig};
use std::fs;
use std::path::{Path, PathBuf};

/// Covered shells and their snapshot file name.
const SHELLS: [(&str, &str); 4] = [
    ("bash", "bash"),
    ("zsh", "zsh"),
    ("fish", "fish"),
    ("powershell", "powershell"),
];

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Compares `actual` with the snapshot file `name`, or writes it if
/// `NVSN_BLESS` is set.
fn check(name: &str, actual: &str) {
    let path = golden_dir().join(name);
    if std::env::var_os("NVSN_BLESS").is_some() {
        fs::create_dir_all(golden_dir()).unwrap_or_else(|e| panic!("create golden dir: {e}"));
        fs::write(&path, actual).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}); run with NVSN_BLESS=1",
            path.display()
        )
    });
    assert_eq!(
        normalize(actual),
        normalize(&expected),
        "snapshot {name} differs; run with NVSN_BLESS=1 if the change is intended"
    );
}

fn input_ctx() -> EnvContext<'static> {
    EnvContext {
        nvsn_dir: Path::new("/home/jo/Mi Dir/.nvsn"),
        active_bin: Some(Path::new("/home/jo/Mi Dir/.nvsn/versions/v20.11.1/bin")),
        node_version: Some("v20.11.1"),
    }
}

fn shell(name: &str) -> Box<dyn ShellConfig> {
    from_str(name).unwrap_or_else(|e| panic!("from_str({name}): {e}"))
}

#[test]
fn env_scripts_match_golden_snapshots() {
    for (name, file) in SHELLS {
        let sh = shell(name);
        check(&format!("{file}.env"), &sh.env_script(&input_ctx()));
    }
}

#[test]
fn wrapper_functions_match_golden_snapshots() {
    for (name, file) in SHELLS {
        let sh = shell(name);
        check(&format!("{file}.wrapper"), sh.wrapper_function());
    }
}
