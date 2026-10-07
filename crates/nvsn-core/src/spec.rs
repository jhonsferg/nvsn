//! Version specifications (ADR-009, ADR-024, ADR-025) and their resolution
//! against the remote index.
//!
//! Decision order (ADR-009): reserved words, then user aliases (`default`
//! included), then number or range. This module covers the reserved words,
//! numbers and ranges. User aliases are not resolved here: [`parse_spec`]
//! returns [`VersionSpec::Alias`] and the store decides.
//!
//! Pending decisions to verify against `nvm.sh` (R-01, R-23):
//! - `lts/-N` counts distinct LTS lines per major, starting from the newest
//!   (`lts/-1` is the LTS before `lts/*`). `lts/-0` is rejected.
//! - `stable` and `unstable` are explicit errors (ADR-025), not aliases.

use crate::remote::RemoteRelease;
use anyhow::{anyhow, bail, Context, Result};
use semver::{Version, VersionReq};
use std::collections::{BTreeSet, HashSet};

/// Version specification as written by the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSpec {
    /// Exact version: `v20.11.1` or `20.11.1`. May carry a prerelease.
    Exact(Version),
    /// Partial version: `20` (major) or `20.11` (major and minor).
    Partial {
        /// Major number.
        major: u64,
        /// Minor number, if given.
        minor: Option<u64>,
    },
    /// Semver range: `>=18 <21`, `^20`, `20.x`.
    Range(VersionReq),
    /// `lts/*` or `lts`: the highest LTS.
    LtsLatest,
    /// `lts/<codename>`, in lowercase. Comparison is case-insensitive.
    LtsCodename(String),
    /// `lts/-N`: the N-th LTS before the most recent one (N >= 1).
    LtsOffset(usize),
    /// `node` or `latest`: the latest published stable version.
    Latest,
    /// `system`: the node installed outside nvsn. Resolved by the caller.
    System,
    /// `current`: the version active in the session. Resolved by the caller.
    Current,
    /// User alias, including `default`. Resolved by the store.
    Alias(String),
}

impl VersionSpec {
    /// Indicates whether the specification is resolved against the remote index.
    pub fn is_remote(&self) -> bool {
        !matches!(
            self,
            VersionSpec::System | VersionSpec::Current | VersionSpec::Alias(_)
        )
    }
}

/// Parses a specification written by the user.
///
/// # Errors
///
/// Returns an error if the string is empty, if it is `iojs` (ADR-024), if it is
/// `stable` or `unstable` (ADR-025), if an `lts/` form is not valid, or if it is
/// not any recognized form.
pub fn parse_spec(input: &str) -> Result<VersionSpec> {
    let input = input.trim();
    if input.is_empty() {
        bail!("version spec is empty");
    }
    let lower = input.to_ascii_lowercase();

    if lower.starts_with("iojs") {
        bail!(
            "io.js is not supported: {input:?} (ADR-024). Use a Node.js version, for example `lts/*`"
        );
    }

    match lower.as_str() {
        "stable" | "unstable" => bail!(
            "{input:?} is obsolete in nvm and nvsn does not resolve it (ADR-025). \
             Use `lts/*` for the latest LTS or `latest` for the latest version"
        ),
        "node" | "latest" => return Ok(VersionSpec::Latest),
        "lts" | "lts/*" => return Ok(VersionSpec::LtsLatest),
        "system" => return Ok(VersionSpec::System),
        "current" => return Ok(VersionSpec::Current),
        _ => {}
    }

    if let Some(rest) = lower.strip_prefix("lts/") {
        return parse_lts_path(rest, input);
    }

    let bare = input
        .strip_prefix('v')
        .or_else(|| input.strip_prefix('V'))
        .unwrap_or(input);
    if let Ok(version) = Version::parse(bare) {
        return Ok(VersionSpec::Exact(version));
    }
    if let Some(partial) = parse_partial(bare) {
        return Ok(partial);
    }

    if looks_like_range(input) {
        return parse_range(input).map(VersionSpec::Range);
    }

    if is_alias_name(input) {
        return Ok(VersionSpec::Alias(input.to_string()));
    }

    bail!(
        "invalid version spec: {input:?}. Formats: v20.11.1, 20, 20.11, lts/*, lts/<codename>, lts/-N, latest, semver ranges"
    )
}

/// Parses the rest of `lts/...` (already lowercase). `original` is only for messages.
fn parse_lts_path(rest: &str, original: &str) -> Result<VersionSpec> {
    if let Some(number) = rest.strip_prefix('-') {
        let n: usize = number
            .parse()
            .with_context(|| format!("{original:?}: expected lts/-N with an integer N"))?;
        if n == 0 {
            bail!("{original:?}: lts/-0 is not valid, use lts/* for the latest LTS");
        }
        return Ok(VersionSpec::LtsOffset(n));
    }
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_alphanumeric()) {
        bail!("{original:?}: unknown LTS codename");
    }
    Ok(VersionSpec::LtsCodename(rest.to_string()))
}

/// Recognizes `20` and `20.11` (digits only, one or two components).
fn parse_partial(bare: &str) -> Option<VersionSpec> {
    let parts: Vec<&str> = bare.split('.').collect();
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    if !parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts.get(1).map(|p| p.parse::<u64>()).transpose().ok()?;
    Some(VersionSpec::Partial { major, minor })
}

/// Indicates whether the string looks like a range and not an alias.
fn looks_like_range(input: &str) -> bool {
    input
        .chars()
        .any(|c| matches!(c, '<' | '>' | '=' | '^' | '~' | '*' | '|' | ',') || c.is_whitespace())
        || input.split('.').any(|p| p == "x" || p == "X")
}

/// Converts Node's range syntax (`>=18 <21`, `>= 18`) to that of
/// `semver`, which separates comparators with commas.
fn parse_range(input: &str) -> Result<VersionReq> {
    if input.contains("||") {
        bail!("{input:?}: ranges with || are not supported");
    }
    let mut comparators: Vec<String> = Vec::new();
    let mut pending = String::new();
    for token in input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
    {
        if matches!(token, ">=" | "<=" | ">" | "<" | "=" | "^" | "~") {
            pending.push_str(token);
            continue;
        }
        comparators.push(format!("{pending}{token}"));
        pending.clear();
    }
    if !pending.is_empty() {
        bail!("{input:?}: the range ends with an operator and no version");
    }
    VersionReq::parse(&comparators.join(", "))
        .with_context(|| format!("invalid semver range: {input:?}"))
}

/// Indicates whether the string can be a user alias: starts with a letter and uses
/// only letters, digits, `-`, `_` and `.`.
fn is_alias_name(input: &str) -> bool {
    // A lone `v` is a version prefix without a number, not an alias.
    if input.eq_ignore_ascii_case("v") {
        return false;
    }
    let mut chars = input.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Resolves a remote specification against the index.
///
/// `releases` may come in any order: the maximum is searched by version.
/// Prerelease versions are only returned by [`VersionSpec::Exact`]
/// and [`VersionSpec::Range`] (semver decides whether the range admits them).
///
/// # Errors
///
/// Returns an error if the specification cannot be resolved against the index
/// (`System`, `Current`, `Alias`), or if no release matches it.
pub fn resolve<'a>(spec: &VersionSpec, releases: &'a [RemoteRelease]) -> Result<&'a RemoteRelease> {
    let found = match spec {
        VersionSpec::Exact(version) => releases.iter().find(|r| &r.version == version),
        VersionSpec::Partial { major, minor } => highest(releases.iter().filter(|r| {
            r.version.pre.is_empty()
                && r.version.major == *major
                && minor.is_none_or(|m| r.version.minor == m)
        })),
        VersionSpec::Range(req) => highest(releases.iter().filter(|r| req.matches(&r.version))),
        VersionSpec::Latest => highest(releases.iter().filter(|r| r.version.pre.is_empty())),
        VersionSpec::LtsLatest => highest(releases.iter().filter(|r| r.lts.is_lts())),
        VersionSpec::LtsCodename(name) => {
            let found = highest(releases.iter().filter(|r| {
                r.lts
                    .codename()
                    .is_some_and(|c| c.eq_ignore_ascii_case(name))
            }));
            if found.is_none() {
                return Err(anyhow!(
                    "no LTS named {name:?} in the index. Known codenames: {}",
                    known_codenames(releases)
                ));
            }
            found
        }
        VersionSpec::LtsOffset(n) => {
            // LTS majors from highest to lowest: lts/* is 0 and lts/-N is N.
            let majors: Vec<u64> = lts_majors(releases).into_iter().rev().collect();
            match majors.get(*n) {
                Some(major) => highest(
                    releases
                        .iter()
                        .filter(|r| r.lts.is_lts() && r.version.major == *major),
                ),
                None => {
                    return Err(anyhow!(
                        "lts/-{n} does not exist: the index has {} LTS line(s)",
                        majors.len()
                    ))
                }
            }
        }
        VersionSpec::System | VersionSpec::Current => {
            bail!("{spec:?} cannot be resolved against the index: the caller must handle it")
        }
        VersionSpec::Alias(name) => {
            bail!("the alias {name:?} cannot be resolved against the index: the store resolves it")
        }
    };

    found.ok_or_else(|| anyhow!("no release in the index matches {}", describe(spec)))
}

/// Returns the release with the highest version among those that pass the filter.
fn highest<'a>(iter: impl Iterator<Item = &'a RemoteRelease>) -> Option<&'a RemoteRelease> {
    iter.max_by(|a, b| a.version.cmp(&b.version))
}

/// Majors with at least one LTS release, from lowest to highest.
fn lts_majors(releases: &[RemoteRelease]) -> BTreeSet<u64> {
    releases
        .iter()
        .filter(|r| r.lts.is_lts())
        .map(|r| r.version.major)
        .collect()
}

/// Distinct LTS codenames in the index, for error messages.
fn known_codenames(releases: &[RemoteRelease]) -> String {
    let mut seen = HashSet::new();
    let mut names: Vec<String> = releases
        .iter()
        .filter_map(|r| r.lts.codename())
        .filter(|name| seen.insert(name.to_ascii_lowercase()))
        .map(str::to_string)
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// Short description of a specification for error messages.
fn describe(spec: &VersionSpec) -> String {
    match spec {
        VersionSpec::Exact(v) => format!("v{v}"),
        VersionSpec::Partial { major, minor: None } => format!("{major}"),
        VersionSpec::Partial {
            major,
            minor: Some(m),
        } => format!("{major}.{m}"),
        VersionSpec::Range(req) => format!("the range {req}"),
        VersionSpec::LtsLatest => "lts/*".to_string(),
        VersionSpec::LtsCodename(name) => format!("lts/{name}"),
        VersionSpec::LtsOffset(n) => format!("lts/-{n}"),
        VersionSpec::Latest => "latest".to_string(),
        VersionSpec::System => "system".to_string(),
        VersionSpec::Current => "current".to_string(),
        VersionSpec::Alias(name) => name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::parse_index;

    /// Test index: two active LTS lines (24 Krypton, 22 Jod), an older one
    /// (20 Iron), an even older one (18 Hydrogen), releases without LTS (26 and 23)
    /// and a prerelease.
    const INDEX: &str = r#"[
        {"version":"v26.0.0","date":"2026-10-01","files":["linux-x64"],"lts":false},
        {"version":"v24.21.0","date":"2026-09-07","files":["linux-x64"],"lts":"Krypton"},
        {"version":"v24.0.0","date":"2025-05-06","files":["linux-x64"],"lts":"Krypton"},
        {"version":"v27.0.0-rc.1","date":"2026-10-05","files":["linux-x64"],"lts":false},
        {"version":"v24.0.0-rc.1","date":"2025-04-01","files":["linux-x64"],"lts":false},
        {"version":"v23.0.0","date":"2024-10-16","files":["linux-x64"],"lts":false},
        {"version":"v22.23.3","date":"2026-09-23","files":["linux-x64"],"lts":"Jod"},
        {"version":"v22.0.0","date":"2024-04-24","files":["linux-x64"],"lts":"Jod"},
        {"version":"v20.20.2","date":"2026-03-24","files":["linux-x64"],"lts":"Iron"},
        {"version":"v20.11.1","date":"2024-02-14","files":["linux-x64"],"lts":"Iron"},
        {"version":"v18.20.8","date":"2025-03-27","files":["linux-x64"],"lts":"Hydrogen"}
    ]"#;

    fn releases() -> Vec<RemoteRelease> {
        parse_index(INDEX).unwrap()
    }

    fn tag(spec: &str) -> String {
        let releases = releases();
        resolve(&parse_spec(spec).unwrap(), &releases)
            .unwrap()
            .tag()
    }

    fn fails(spec: &str) -> String {
        let releases = releases();
        match parse_spec(spec).and_then(|s| resolve(&s, &releases).map(|r| r.tag())) {
            Ok(tag) => panic!("{spec:?} should fail but gave {tag}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn exact_with_and_without_prefix() {
        assert_eq!(tag("v20.11.1"), "v20.11.1");
        assert_eq!(tag("20.11.1"), "v20.11.1");
        assert_eq!(tag("V20.11.1"), "v20.11.1");
        assert_eq!(tag("24.0.0-rc.1"), "v24.0.0-rc.1");
    }

    #[test]
    fn exact_missing_is_error() {
        assert!(fails("v20.0.1").contains("no release in the index matches"));
    }

    #[test]
    fn partial_picks_highest_matching_stable() {
        assert_eq!(tag("20"), "v20.20.2");
        assert_eq!(tag("20.11"), "v20.11.1");
        assert_eq!(tag("24"), "v24.21.0");
        assert!(fails("99").contains("99"));
    }

    #[test]
    fn partial_and_range_never_pick_a_prerelease_alone() {
        assert_eq!(tag("26"), "v26.0.0");
        assert!(fails("27").contains("27"));
        assert!(fails(">=27").contains(">=27"));
        assert_eq!(tag("27.0.0-rc.1"), "v27.0.0-rc.1");
    }

    #[test]
    fn lts_latest_and_lts_word() {
        assert_eq!(tag("lts/*"), "v24.21.0");
        assert_eq!(tag("LTS"), "v24.21.0");
    }

    #[test]
    fn lts_codename_is_case_insensitive() {
        assert_eq!(tag("lts/hydrogen"), "v18.20.8");
        assert_eq!(tag("lts/JOD"), "v22.23.3");
        assert_eq!(tag("lts/Iron"), "v20.20.2");
    }

    #[test]
    fn lts_unknown_codename_lists_known_ones() {
        let msg = fails("lts/lithium");
        assert!(msg.contains("lithium"));
        assert!(msg.contains("Krypton"));
        assert!(msg.contains("Hydrogen"));
    }

    #[test]
    fn lts_offset_counts_lts_lines_from_newest() {
        assert_eq!(tag("lts/-1"), "v22.23.3");
        assert_eq!(tag("lts/-2"), "v20.20.2");
        assert_eq!(tag("lts/-3"), "v18.20.8");
    }

    #[test]
    fn lts_offset_out_of_range_is_error() {
        assert!(fails("lts/-4").contains("lts/-4"));
        assert!(parse_spec("lts/-0").is_err());
        assert!(parse_spec("lts/-x").is_err());
    }

    #[test]
    fn latest_and_node_are_newest_stable() {
        // v27.0.0-rc.1 is higher but is a prerelease: it is skipped.
        assert_eq!(tag("latest"), "v26.0.0");
        assert_eq!(tag("node"), "v26.0.0");
    }

    #[test]
    fn semver_ranges_with_node_syntax() {
        assert_eq!(tag(">=18 <21"), "v20.20.2");
        assert_eq!(tag(">= 18, < 21"), "v20.20.2");
        assert_eq!(tag("^22"), "v22.23.3");
        assert_eq!(tag("20.x"), "v20.20.2");
    }

    #[test]
    fn range_without_match_is_error() {
        assert!(fails(">=40").contains(">=40"));
    }

    #[test]
    fn range_with_dangling_operator_is_error() {
        assert!(parse_spec(">=18 <").is_err());
        assert!(parse_spec(">=18 || <21").is_err());
    }

    #[test]
    fn iojs_is_rejected_with_clear_message() {
        let err = parse_spec("iojs").unwrap_err().to_string();
        assert!(err.contains("io.js"));
        assert!(err.contains("ADR-024"));
        assert!(parse_spec("iojs-v3").is_err());
    }

    #[test]
    fn stable_and_unstable_are_explicit_errors() {
        for word in ["stable", "unstable", "STABLE"] {
            let err = parse_spec(word).unwrap_err().to_string();
            assert!(err.contains("ADR-025"), "{word}: {err}");
        }
    }

    #[test]
    fn user_alias_and_default_are_left_to_the_store() {
        assert_eq!(
            parse_spec("default").unwrap(),
            VersionSpec::Alias("default".to_string())
        );
        assert_eq!(
            parse_spec("my-project").unwrap(),
            VersionSpec::Alias("my-project".to_string())
        );
        let releases = releases();
        let err = resolve(&parse_spec("default").unwrap(), &releases)
            .unwrap_err()
            .to_string();
        assert!(err.contains("store"));
    }

    #[test]
    fn system_and_current_are_not_remote() {
        assert_eq!(parse_spec("system").unwrap(), VersionSpec::System);
        assert_eq!(parse_spec("current").unwrap(), VersionSpec::Current);
        assert!(!VersionSpec::System.is_remote());
        assert!(VersionSpec::LtsLatest.is_remote());
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        assert!(parse_spec("").is_err());
        assert!(parse_spec("   ").is_err());
        assert!(parse_spec("v").is_err());
        assert!(parse_spec("20.11.1.1").is_err());
        assert!(parse_spec("lts/").is_err());
        assert!(parse_spec("lts/-").is_err());
    }

    #[test]
    fn resolve_is_independent_of_input_order() {
        let mut shuffled = releases();
        shuffled.reverse();
        let spec = parse_spec("lts/*").unwrap();
        assert_eq!(resolve(&spec, &shuffled).unwrap().tag(), "v24.21.0");
    }
}
