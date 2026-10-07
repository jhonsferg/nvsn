//! `outdated`: compares each installed version with the latest release of the same
//! `major.minor` in the Node index.
//!
//! It uses the index from the cache (refreshed only if expired, using its TTL). It
//! does not download any Node version.

use crate::ctx::Ctx;
use crate::exit::CliError;
use crate::versions::list_installed;
use nvsn_core::RemoteRelease;
use semver::Version;
use serde_json::{json, Value};

/// Status of an installed version compared with the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// It is the latest release of its `major.minor` line, or newer.
    UpToDate,
    /// There are later releases in the same `major.minor` line.
    Behind {
        /// Number of patches behind the latest release of the line.
        patches: u64,
    },
    /// The index has no release of that `major.minor` line.
    Unknown,
}

/// Result of comparing an installed version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Installed tag, e.g. `v20.11.1`.
    pub installed: String,
    /// Latest release of the line, if the index has it.
    pub latest: Option<String>,
    /// Status compared with the index.
    pub status: Status,
}

/// Shows the status of the installed versions.
///
/// # Errors
///
/// Cache or network error (codes 5 or 7) if the index cannot be obtained.
pub fn outdated(ctx: &Ctx) -> Result<(), CliError> {
    let installed = list_installed(&ctx.store)?;
    if installed.is_empty() {
        ctx.out.result(
            "outdated",
            "No Node.js versions installed. Run `nvsn install 20`.",
            json!({ "versions": [] }),
        );
        return Ok(());
    }

    let index = ctx.load_index()?;
    let entries: Vec<Entry> = installed
        .iter()
        .map(|inst| classify(&inst.version, &index))
        .collect();

    let rows: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let (status, patches) = match entry.status {
                Status::UpToDate => ("up_to_date", None),
                Status::Behind { patches } => ("behind", Some(patches)),
                Status::Unknown => ("unknown", None),
            };
            json!({
                "installed": entry.installed,
                "latest": entry.latest,
                "status": status,
                "patches_behind": patches,
            })
        })
        .collect();

    let human = render(&entries);
    ctx.out
        .result("outdated", &human, json!({ "versions": rows }));
    Ok(())
}

/// Classifies an installed version against the releases of the index.
///
/// Takes the highest release of the same `major.minor` as the installed version.
pub fn classify(installed: &Version, index: &[RemoteRelease]) -> Entry {
    let latest = index
        .iter()
        .map(|release| &release.version)
        .filter(|v| v.major == installed.major && v.minor == installed.minor)
        .max();

    let label = format!("v{installed}");
    match latest {
        None => Entry {
            installed: label,
            latest: None,
            status: Status::Unknown,
        },
        Some(latest) if installed >= latest => Entry {
            installed: label,
            latest: Some(format!("v{latest}")),
            status: Status::UpToDate,
        },
        Some(latest) => Entry {
            installed: label,
            latest: Some(format!("v{latest}")),
            status: Status::Behind {
                patches: latest.patch.saturating_sub(installed.patch),
            },
        },
    }
}

/// Table text: one row per version and a final summary.
fn render(entries: &[Entry]) -> String {
    let mut lines = vec![format!(
        "  {:<14} {:<14} {}",
        "Installed", "Latest patch", "Status"
    )];
    for entry in entries {
        let latest = entry.latest.as_deref().unwrap_or("-");
        let status = match entry.status {
            Status::UpToDate => "up to date".to_owned(),
            Status::Behind { patches } => {
                format!(
                    "{patches} patch{} behind",
                    if patches == 1 { "" } else { "es" }
                )
            }
            Status::Unknown => "no data in the index".to_owned(),
        };
        lines.push(format!("  {:<14} {:<14} {status}", entry.installed, latest));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvsn_core::remote::LtsField;

    fn v(text: &str) -> Version {
        Version::parse(text).expect("valid version")
    }

    fn release(text: &str) -> RemoteRelease {
        RemoteRelease {
            version: v(text),
            date: String::new(),
            files: Vec::new(),
            npm: None,
            lts: LtsField::Flag(false),
            security: false,
        }
    }

    #[test]
    fn same_version_is_up_to_date() {
        let index = [release("20.11.1")];
        let entry = classify(&v("20.11.1"), &index);
        assert_eq!(entry.status, Status::UpToDate);
        assert_eq!(entry.latest.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn counts_patches_within_the_same_minor() {
        let index = [release("20.11.0"), release("20.11.1"), release("20.12.0")];
        let entry = classify(&v("20.11.0"), &index);
        assert_eq!(entry.status, Status::Behind { patches: 1 });
        assert_eq!(entry.latest.as_deref(), Some("v20.11.1"));
    }

    #[test]
    fn ignores_other_minor_lines() {
        let index = [release("20.12.0"), release("20.12.5")];
        let entry = classify(&v("20.11.0"), &index);
        assert_eq!(entry.status, Status::Unknown);
        assert_eq!(entry.latest, None);
    }

    #[test]
    fn installed_newer_than_index_is_up_to_date() {
        let index = [release("20.11.0")];
        assert_eq!(classify(&v("20.11.2"), &index).status, Status::UpToDate);
    }

    #[test]
    fn render_uses_singular_and_plural() {
        let one = Entry {
            installed: "v20.11.0".into(),
            latest: Some("v20.11.1".into()),
            status: Status::Behind { patches: 1 },
        };
        let many = Entry {
            installed: "v18.1.0".into(),
            latest: Some("v18.3.0".into()),
            status: Status::Behind { patches: 2 },
        };
        let text = render(&[one, many]);
        assert!(text.contains("1 patch behind"));
        assert!(text.contains("2 patches behind"));
    }
}
