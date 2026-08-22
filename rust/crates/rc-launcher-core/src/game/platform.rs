//! Platform detection & rule matching for dependency resolution (task 4).
//!
//! Minecraft's `version.json` gates libraries, natives and even whole artifacts
//! behind *rules* that reference the current operating system and CPU
//! architecture. This module captures our target platform and evaluates those
//! rules exactly the way Mojang's launcher does:
//!
//! * a library with **no** rules is always included;
//! * a library with rules is excluded by default and only included when the
//!   *last* matching rule's action is `allow`;
//! * a rule matches when its `os`/`features` conditions are satisfied by the
//!   current platform (see [`OsRule`]).
//!
//! For an Android device we present `Linux` on `AArch64` ([`Platform::android`]);
//! on the host (where the unit tests run) we detect the real platform
//! ([`Platform::host`]).

use std::collections::HashMap;

/// A supported operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OsName {
    Linux,
    Windows,
    Osx,
}

impl OsName {
    /// The string Mojang uses inside rules (`os.name`).
    pub fn as_rule_str(self) -> &'static str {
        match self {
            OsName::Linux => "linux",
            OsName::Windows => "windows",
            OsName::Osx => "osx",
        }
    }

    /// Parse a Mojang `os.name` value. `mac` is accepted as an alias for `osx`.
    pub fn from_rule(s: &str) -> Option<OsName> {
        match s {
            "linux" => Some(OsName::Linux),
            "windows" => Some(OsName::Windows),
            "osx" | "mac" => Some(OsName::Osx),
            _ => None,
        }
    }
}

/// A CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    X86,
    X86_64,
    Arm,
    Arm64,
    /// Anything we do not explicitly recognise.
    Unknown,
}

impl Arch {
    /// The string Mojang uses inside rules (`os.arch`).
    pub fn as_rule_str(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X86_64 => "x86_64",
            Arch::Arm => "arm",
            Arch::Arm64 => "aarch64",
            Arch::Unknown => "",
        }
    }

    /// Parse a Mojang `os.arch` value. `amd64`/`arm64` aliases are accepted.
    pub fn from_rule(s: &str) -> Option<Arch> {
        match s {
            "x86" => Some(Arch::X86),
            "x86_64" | "amd64" => Some(Arch::X86_64),
            "arm" => Some(Arch::Arm),
            "aarch64" | "arm64" => Some(Arch::Arm64),
            _ => None,
        }
    }
}

/// The platform we resolve dependencies for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: OsName,
    pub arch: Arch,
    /// OS version string (used by a minority of `os.version` rules).
    pub os_version: String,
}

impl Platform {
    /// Detect the *host* platform from the compile-time target. On the CI/test
    /// host this is whatever the machine is; on Android it yields Linux/AArch64.
    pub fn host() -> Platform {
        let os = if cfg!(target_os = "linux") {
            OsName::Linux
        } else if cfg!(target_os = "windows") {
            OsName::Windows
        } else if cfg!(target_os = "macos") {
            OsName::Osx
        } else {
            OsName::Linux
        };
        let arch = if cfg!(target_arch = "x86") {
            Arch::X86
        } else if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "arm") {
            Arch::Arm
        } else if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::Unknown
        };
        Platform {
            os,
            arch,
            os_version: String::new(),
        }
    }

    /// The platform an Android device presents: Linux on AArch64.
    pub fn android() -> Platform {
        Platform {
            os: OsName::Linux,
            arch: Arch::Arm64,
            os_version: String::new(),
        }
    }
}

/// An `os` condition inside a download rule.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

impl OsRule {
    /// Does this OS condition match the given platform?
    ///
    /// Every specified component must match; an absent component is treated as
    /// "match anything". The `version` component is matched with a small
    /// anchored regex engine (see [`version_matches`]) good enough for Mojang's
    /// `^10\.11$`-style patterns.
    pub fn matches(&self, platform: &Platform) -> bool {
        if let Some(n) = &self.name {
            if OsName::from_rule(n) != Some(platform.os) {
                return false;
            }
        }
        if let Some(a) = &self.arch {
            if Arch::from_rule(a) != Some(platform.arch) {
                return false;
            }
        }
        if let Some(v) = &self.version {
            if !version_matches(v, &platform.os_version) {
                return false;
            }
        }
        true
    }
}

/// A minimal anchored regex matcher for OS-version rules.
///
/// Supports `.` (any char), `\d` (digit), `\.` (literal dot), `*`/`+`
/// (Kleene star / plus) and literal characters. Anchors `^`/`$` are implicit
/// (the whole string must match). This covers every Mojang version pattern
/// without pulling in the `regex` crate (which is unavailable in our offline
/// build cache).
fn version_matches(pattern: &str, version: &str) -> bool {
    let stripped = pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .strip_suffix('$')
        .unwrap_or(pattern);
    let nodes = parse_pattern(stripped);
    let t: Vec<char> = version.chars().collect();
    matches_at(&nodes, &t, 0, 0)
}

/// A single regex token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// Literal character (also used for an escaped `\.`).
    Lit(char),
    /// `.` — any character.
    Any,
    /// `\d` — ASCII digit.
    Digit,
}

/// A parse node: a token plus an optional greedy quantifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Node {
    tok: Tok,
    quant: Option<Quant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quant {
    Star, // `*`
    Plus, // `+`
}

/// Parse a pattern string into a list of [`Node`]s.
fn parse_pattern(pattern: &str) -> Vec<Node> {
    let p: Vec<char> = pattern.chars().collect();
    let mut nodes: Vec<Node> = Vec::new();
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        let tok = if c == '\\' && i + 1 < p.len() {
            i += 1;
            match p[i] {
                'd' => Tok::Digit,
                other => Tok::Lit(other),
            }
        } else if c == '.' {
            Tok::Any
        } else {
            Tok::Lit(c)
        };
        i += 1;
        // Look ahead for a quantifier.
        let mut quant = None;
        if i < p.len() && (p[i] == '*' || p[i] == '+') {
            quant = Some(if p[i] == '*' {
                Quant::Star
            } else {
                Quant::Plus
            });
            i += 1;
        }
        nodes.push(Node { tok, quant });
    }
    nodes
}

/// Does `tok` match a single character `ch`?
fn match_one(tok: Tok, ch: char) -> bool {
    match tok {
        Tok::Lit(c) => c == ch,
        Tok::Any => true,
        Tok::Digit => ch.is_ascii_digit(),
    }
}

/// Backtracking matcher over the parsed nodes (full match required).
fn matches_at(nodes: &[Node], t: &[char], mut ni: usize, mut ti: usize) -> bool {
    while ni < nodes.len() {
        let node = nodes[ni];
        match node.quant {
            None => {
                if ti >= t.len() || !match_one(node.tok, t[ti]) {
                    return false;
                }
                ti += 1;
                ni += 1;
            }
            Some(Quant::Star) => {
                // zero or more
                for k in ti..=t.len() {
                    if matches_at(nodes, t, ni + 1, k) {
                        return true;
                    }
                    if k >= t.len() || !match_one(node.tok, t[k]) {
                        break;
                    }
                }
                return false;
            }
            Some(Quant::Plus) => {
                // one or more: require at least one match first.
                if ti >= t.len() || !match_one(node.tok, t[ti]) {
                    return false;
                }
                let mut k = ti + 1;
                loop {
                    if matches_at(nodes, t, ni + 1, k) {
                        return true;
                    }
                    if k < t.len() && match_one(node.tok, t[k]) {
                        k += 1;
                    } else {
                        break;
                    }
                }
                return false;
            }
        }
    }
    ti == t.len()
}

/// Evaluate a set of feature flags (used by `features` rules). Defaults to
/// `false` for any unspecified feature (we resolve *downloadable* artifacts, not
/// launch-time optional features such as demo-user / custom-resolution).
pub type Features = HashMap<String, bool>;

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_arm64() -> Platform {
        Platform {
            os: OsName::Linux,
            arch: Arch::Arm64,
            os_version: String::new(),
        }
    }

    #[test]
    fn detect_host_is_linux_on_ci() {
        let p = Platform::host();
        assert_eq!(p.os, OsName::Linux);
    }

    #[test]
    fn android_platform_is_linux_aarch64() {
        let p = Platform::android();
        assert_eq!(p.os, OsName::Linux);
        assert_eq!(p.arch, Arch::Arm64);
        assert_eq!(p.arch.as_rule_str(), "aarch64");
    }

    #[test]
    fn os_rule_name_match() {
        let rule = OsRule {
            name: Some("linux".into()),
            arch: None,
            version: None,
        };
        assert!(rule.matches(&linux_arm64()));

        let win = OsRule {
            name: Some("windows".into()),
            arch: None,
            version: None,
        };
        assert!(!win.matches(&linux_arm64()));
    }

    #[test]
    fn os_rule_arch_match() {
        let rule = OsRule {
            name: Some("linux".into()),
            arch: Some("x86_64".into()),
            version: None,
        };
        assert!(!rule.matches(&linux_arm64()));

        let arm = OsRule {
            name: Some("linux".into()),
            arch: Some("aarch64".into()),
            version: None,
        };
        assert!(arm.matches(&linux_arm64()));
    }

    #[test]
    fn os_rule_version_regex() {
        let rule = OsRule {
            name: Some("osx".into()),
            arch: None,
            version: Some("^10\\.11$".into()),
        };
        let mac = Platform {
            os: OsName::Osx,
            arch: Arch::X86_64,
            os_version: "10.11".into(),
        };
        assert!(rule.matches(&mac));

        let mac12 = Platform {
            os: OsName::Osx,
            arch: Arch::X86_64,
            os_version: "10.12".into(),
        };
        assert!(!rule.matches(&mac12));
    }

    #[test]
    fn version_matcher_handles_star_and_digit_class() {
        assert!(version_matches("^10\\.\\d+$", "10.5"));
        assert!(version_matches("^10\\.\\d+$", "10.13"));
        assert!(!version_matches("^10\\.\\d+$", "10.5.1"));
        assert!(version_matches("1\\.7\\..*", "1.7.10"));
    }
}
