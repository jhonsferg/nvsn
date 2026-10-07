//! Injectable access to environment variables.
//!
//! Platform and path detection read the environment through
//! [`EnvSource`], so it can be tested without touching the process's real variables.

/// Environment variable source. The real implementation is [`ProcessEnv`].
pub trait EnvSource: Send + Sync {
    /// Returns the value of `key`, or `None` if it is not defined or is empty.
    fn var(&self, key: &str) -> Option<String>;
}

/// The real process environment (`std::env`).
///
/// Empty variables are treated as undefined: `NVSN_DIR=` must not
/// produce an empty path.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }
}

/// Fake environment for tests: a fixed map of variables.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MapEnv(std::collections::BTreeMap<String, String>);

#[cfg(test)]
impl MapEnv {
    /// Builds the environment from `(key, value)` pairs.
    pub(crate) fn new(pairs: &[(&str, &str)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        )
    }
}

#[cfg(test)]
impl EnvSource for MapEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}
