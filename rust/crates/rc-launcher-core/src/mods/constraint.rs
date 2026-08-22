//! Version-constraint model for mod dependencies (task 8).
//!
//! Minecraft mods express version requirements in two flavours:
//!
//! * **Fabric / Quilt** use standard [semver ranges](https://semver.org):
//!   `>=1.14`, `~1.16`, `1.16.5`, `*` ...
//! * **Forge** uses Maven-style interval notation inside `mods.toml` /
//!   `mcmod.info`: `[1.16.5]` (exact), `[24,)` (`>=24`), `(,1.12.2]`
//!   (`<=1.12.2`), `[1.0,2.0)` (`>=1.0,<2.0`) ...
//!
//! [`VersionConstraint`] normalises both into a [`semver::VersionReq`] and
//! offers [`VersionConstraint::matches`] that knows how to coerce a concrete
//! Minecraft version (`1.16.5`, `1.7.10`) into something `semver` can compare.

use crate::error::RcResult;
use std::fmt;

/// A parsed dependency / compatibility constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionConstraint {
    raw: String,
    req: semver::VersionReq,
}

impl VersionConstraint {
    /// Parse a constraint, transparently converting Maven/Forge interval
    /// notation into an equivalent semver requirement.
    pub fn parse(s: &str) -> RcResult<Self> {
        let raw = s.trim().to_string();
        if raw.is_empty() {
            return Err(crate::error::RcError::Other(
                "empty version constraint".into(),
            ));
        }
        let semver_str = convert_maven_range(&raw);
        let req = semver::VersionReq::parse(&semver_str).map_err(|e| {
            crate::error::RcError::Other(format!("invalid version constraint '{raw}': {e}"))
        })?;
        Ok(Self { raw, req })
    }

    /// The original, unnormalised constraint text.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Does `version` (a concrete Minecraft version such as `1.16.5`) satisfy
    /// this constraint? Unknown / unparseable versions are treated as
    /// *non-matching* so a broken manifest fails safe rather than silently
    /// masking an incompatibility.
    pub fn matches(&self, version: &str) -> bool {
        match coerce_mc_version(version) {
            Some(v) => self.req.matches(&v),
            None => false,
        }
    }

    /// Expose the underlying semver requirement (used by tests and FFI).
    pub fn as_version_req(&self) -> &semver::VersionReq {
        &self.req
    }
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Coerce a concrete Minecraft version string into a [`semver::Version`].
///
/// Minecraft versions are always `major.minor.patch` in practice
/// (`1.16.5`, `1.7.10`, `1.20.1`); this also tolerates short forms
/// (`1.16`, `1`) and trailing `-Pre` style qualifiers, mapping the numeric
/// part and dropping the rest.
pub fn coerce_mc_version(s: &str) -> Option<semver::Version> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Strip any non-numeric suffix such as "-pre1" / "-rc2".
    let numeric: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = numeric
        .split('.')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u64>())
        .collect::<Result<Vec<u64>, _>>()
        .ok()?;
    if parts.is_empty() {
        return None;
    }
    // Pad to exactly three components so semver is happy.
    while parts.len() < 3 {
        parts.push(0);
    }
    let (major, minor, patch) = (parts[0], parts[1], parts[2]);
    Some(semver::Version::new(major, minor, patch))
}

/// Translate Maven / Forge interval notation into a semver requirement string.
///
/// `1.16.5` (no brackets) is left untouched and parsed as the semver range
/// `=1.16.5` by the caller's `VersionReq`. Everything else is mapped:
///
/// | Forge        | semver     |
/// |--------------|------------|
/// | `[1.16.5]`   | `=1.16.5`  |
/// | `[24,)`      | `>=24`     |
/// | `(,1.12.2]`  | `<=1.12.2` |
/// | `(1.0,2.0)`  | `>1.0,<2.0`|
/// | `[1.0,2.0)`  | `>=1.0,<2.0`|
fn convert_maven_range(s: &str) -> String {
    let s = s.trim();
    // Bare version (no brackets / comparator) → exact match.
    if !s.contains('[') && !s.contains('(') && !s.contains(',') {
        // Already a semver operator? leave it.
        if s.starts_with(['>', '<', '=', '^', '~', '*']) {
            return s.to_string();
        }
        return format!("={s}");
    }

    // Extract the interval body, e.g. "[1.0,2.0)" → "1.0,2.0".
    let body = s
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')']);
    // A single value with no comma (e.g. `[1.16.5]` / `(1.16.5)`) is an exact
    // pin, not a half-open interval.
    if !body.contains(',') {
        return format!("={body}");
    }
    let (lower_inclusive, upper_inclusive) = (s.starts_with('['), s.ends_with(']'));
    let mut parts = body.split(',').map(|p| p.trim());
    let lower = parts.next().unwrap_or("");
    let upper = parts.next().unwrap_or("");

    let mut out = Vec::new();
    if !lower.is_empty() {
        let cmp = if lower_inclusive { ">=" } else { ">" };
        out.push(format!("{cmp}{lower}"));
    }
    if !upper.is_empty() {
        let cmp = if upper_inclusive { "<=" } else { "<" };
        out.push(format!("{cmp}{upper}"));
    }
    if out.is_empty() {
        // Open interval with nothing inside — matches everything.
        return "*".to_string();
    }
    out.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coerce_versions() {
        assert_eq!(
            coerce_mc_version("1.16.5"),
            Some(semver::Version::new(1, 16, 5))
        );
        assert_eq!(
            coerce_mc_version("1.7.10"),
            Some(semver::Version::new(1, 7, 10))
        );
        assert_eq!(
            coerce_mc_version("1.16"),
            Some(semver::Version::new(1, 16, 0))
        );
        assert_eq!(coerce_mc_version("1"), Some(semver::Version::new(1, 0, 0)));
        assert_eq!(coerce_mc_version("1.20.1-pre2").unwrap().minor, 20);
        assert_eq!(coerce_mc_version(""), None);
        assert_eq!(coerce_mc_version("garbage"), None);
    }

    #[test]
    fn maven_ranges() {
        assert_eq!(convert_maven_range("[1.16.5]"), "=1.16.5");
        assert_eq!(convert_maven_range("[24,)"), ">=24");
        assert_eq!(convert_maven_range("[,1.12.2]"), "<=1.12.2");
        assert_eq!(convert_maven_range("(1.0,2.0)"), ">1.0,<2.0");
        assert_eq!(convert_maven_range("[1.0,2.0)"), ">=1.0,<2.0");
        assert_eq!(convert_maven_range("1.16.5"), "=1.16.5");
        assert_eq!(convert_maven_range(">=1.14"), ">=1.14");
    }

    #[test]
    fn constraint_matches() {
        let c = VersionConstraint::parse("[1.16.5]").unwrap();
        assert!(c.matches("1.16.5"));
        assert!(!c.matches("1.16.4"));

        let c = VersionConstraint::parse("[24,)").unwrap();
        assert!(c.matches("24"));
        assert!(c.matches("40"));
        assert!(!c.matches("23"));

        let c = VersionConstraint::parse(">=1.14").unwrap();
        assert!(c.matches("1.16.5"));
        assert!(!c.matches("1.12.2"));

        let c = VersionConstraint::parse("[1.0,2.0)").unwrap();
        assert!(c.matches("1.5.0"));
        assert!(!c.matches("2.0.0"));
    }

    #[test]
    fn invalid_constraint_errors() {
        assert!(VersionConstraint::parse("").is_err());
        assert!(VersionConstraint::parse("!!bad!!").is_err());
    }
}
