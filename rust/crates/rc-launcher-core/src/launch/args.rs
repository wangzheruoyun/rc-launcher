//! Argument templating & rule-filtered argument lists (task 7).
//!
//! A `version.json` describes its command line as *templates*: either the legacy
//! flat `minecraftArguments` string (MC ≤ 1.12) or the modern `arguments.{game,
//! jvm}` lists whose entries may be plain strings or rule-gated objects
//! (MC ≥ 1.13):
//!
//! ```json
//! { "rules": [{ "action": "allow", "features": { "is_demo_user": true } }],
//!   "value": "--demo" }
//! ```
//!
//! Both forms embed `${placeholder}` variables that the launcher must fill in
//! (`${auth_player_name}`, `${classpath}`, `${natives_directory}`, …). This
//! module implements the three primitives the command builder needs, mirroring
//! FCLCore's `Argument`/`StringUtils.formatVersion` handling:
//!
//! * [`Substitutions`] — the placeholder table + `${…}` expansion,
//! * [`flatten_arguments`] — rule filtering + `value: string | [string]`,
//! * [`prune_unresolved`] — drop arguments whose placeholder stayed unresolved
//!   (offline accounts have no `${clientid}`/`${auth_xuid}`, for example), so the
//!   game never receives a literal `${clientid}` and dies on argument parsing.

use std::collections::BTreeMap;

use crate::error::{RcError, RcResult};
use crate::game::library::{Action, Rule};
use crate::game::platform::{Features, Platform};

/// Placeholder table used to expand `${…}` templates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Substitutions {
    map: BTreeMap<String, String>,
}

impl Substitutions {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define (or redefine) `key` → `value`.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.map.insert(key.into(), value.into());
        self
    }

    /// Value of `key`, if defined.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    /// Is `key` defined?
    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Number of defined placeholders.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Is the table empty?
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Expand every known `${key}` in `template`.
    ///
    /// Unknown placeholders are left verbatim so [`prune_unresolved`] can drop
    /// the argument instead of handing the game a literal `${…}`. A `${` that is
    /// never closed is also left verbatim (never panics, never loops).
    pub fn apply(&self, template: &str) -> String {
        let bytes = template.as_bytes();
        let mut out = String::with_capacity(template.len());
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                if let Some(end) = template[i + 2..].find('}') {
                    let key = &template[i + 2..i + 2 + end];
                    match self.map.get(key) {
                        Some(v) => out.push_str(v),
                        // keep the placeholder verbatim (pruned later)
                        None => out.push_str(&template[i..i + 2 + end + 1]),
                    }
                    i += 2 + end + 1;
                    continue;
                }
                // unterminated `${` — copy the rest verbatim
                out.push_str(&template[i..]);
                break;
            }
            // copy one UTF-8 character (bytes[i] is a leading byte here)
            let ch_len = utf8_len(bytes[i]);
            let end = (i + ch_len).min(template.len());
            out.push_str(&template[i..end]);
            i = end;
        }
        out
    }

    /// Every `${key}` in `s` that this table cannot expand.
    pub fn unresolved(&self, s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = s;
        while let Some(start) = rest.find("${") {
            let after = &rest[start + 2..];
            match after.find('}') {
                Some(end) => {
                    let key = &after[..end];
                    if !self.map.contains_key(key) {
                        out.push(key.to_string());
                    }
                    rest = &after[end + 1..];
                }
                None => break,
            }
        }
        out
    }
}

/// Length in bytes of the UTF-8 character starting with `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        // continuation / invalid byte: consume one byte to guarantee progress
        _ => 1,
    }
}

/// Does a rule list allow the entry for `platform` / `features`?
///
/// Same semantics as [`crate::game::library::Library::is_allowed`]: no rules =>
/// allowed; otherwise the *last matching* rule decides and the default is deny.
pub fn rules_allow(rules: &[Rule], platform: &Platform, features: &Features) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(platform, features) {
            allowed = rule.action == Action::Allow;
        }
    }
    allowed
}

/// Flatten a modern `arguments.game` / `arguments.jvm` list.
///
/// Accepts each entry as either a bare string or `{ "rules": [...], "value":
/// string | [string] }`; rule-gated entries are dropped when the rules deny
/// them. Anything else is a malformed manifest and reported as such (robustness:
/// a silent skip would produce a subtly broken command line).
pub fn flatten_arguments(
    values: &[serde_json::Value],
    platform: &Platform,
    features: &Features,
) -> RcResult<Vec<String>> {
    let mut out = Vec::new();
    for v in values {
        match v {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Object(obj) => {
                let rules: Vec<Rule> = match obj.get("rules") {
                    Some(r) => serde_json::from_value(r.clone()).map_err(RcError::Json)?,
                    None => Vec::new(),
                };
                if !rules_allow(&rules, platform, features) {
                    continue;
                }
                match obj.get("value") {
                    Some(serde_json::Value::String(s)) => out.push(s.clone()),
                    Some(serde_json::Value::Array(items)) => {
                        for it in items {
                            match it {
                                serde_json::Value::String(s) => out.push(s.clone()),
                                other => {
                                    return Err(RcError::Launch(format!(
                                        "malformed argument value element: {other}"
                                    )))
                                }
                            }
                        }
                    }
                    other => {
                        return Err(RcError::Launch(format!(
                            "malformed argument entry (no usable `value`): {:?}",
                            other
                        )))
                    }
                }
            }
            other => {
                return Err(RcError::Launch(format!(
                    "malformed argument entry: {other}"
                )))
            }
        }
    }
    Ok(out)
}

/// Split a legacy `minecraftArguments` string into arguments.
///
/// The field is whitespace separated; empty tokens are skipped.
pub fn split_legacy_arguments(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

/// Result of [`prune_unresolved`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunedArgs {
    /// Arguments that survived (all placeholders resolved).
    pub args: Vec<String>,
    /// Human-readable notes about what was dropped and why (diagnostics).
    pub dropped: Vec<String>,
}

/// Expand `args` and drop the ones that still contain an unresolved `${…}`.
///
/// When a value is dropped its preceding `--flag` is dropped too, otherwise the
/// game would see a flag with the *next* flag as its value (e.g. `--xuid
/// --clientId abc`), which is far worse than not passing it at all.
pub fn prune_unresolved(args: &[String], subs: &Substitutions) -> PrunedArgs {
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut dropped: Vec<String> = Vec::new();
    for raw in args {
        let expanded = subs.apply(raw);
        let missing = subs.unresolved(&expanded);
        if missing.is_empty() {
            out.push(expanded);
            continue;
        }
        // Drop the value ...
        let mut note = format!("{} (unresolved ${{{}}})", raw, missing.join("}, ${"));
        // ... and its flag, if the previous emitted token was one.
        if let Some(prev) = out.last() {
            if is_flag(prev) {
                note = format!("{} {}", prev, note);
                out.pop();
            }
        }
        dropped.push(note);
    }
    PrunedArgs { args: out, dropped }
}

/// Is `s` a command-line flag (`-x` / `--long`) rather than a value?
fn is_flag(s: &str) -> bool {
    s.starts_with('-') && s.len() > 1
}

/// Does `args` already contain `flag` (exactly, or as `flag=value`)?
pub fn has_flag(args: &[String], flag: &str) -> bool {
    let prefix = format!("{}=", flag);
    args.iter().any(|a| a == flag || a.starts_with(&prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::platform::{Arch, OsName};

    fn android() -> Platform {
        Platform {
            os: OsName::Linux,
            arch: Arch::Arm64,
            os_version: String::new(),
        }
    }

    fn subs() -> Substitutions {
        let mut s = Substitutions::new();
        s.set("auth_player_name", "Steve")
            .set("version_name", "1.20.4")
            .set("classpath", "/a.jar:/b.jar");
        s
    }

    #[test]
    fn expands_known_placeholders() {
        let s = subs();
        assert_eq!(s.apply("${auth_player_name}"), "Steve");
        assert_eq!(
            s.apply("--username ${auth_player_name} --version ${version_name}"),
            "--username Steve --version 1.20.4"
        );
        // adjacent placeholders and surrounding text
        assert_eq!(s.apply("[${version_name}]"), "[1.20.4]");
        assert_eq!(s.apply("${version_name}${version_name}"), "1.20.41.20.4");
    }

    #[test]
    fn keeps_unknown_placeholders_verbatim() {
        let s = subs();
        assert_eq!(s.apply("${clientid}"), "${clientid}");
        assert_eq!(s.unresolved("${clientid}"), vec!["clientid".to_string()]);
        assert!(s.unresolved("${version_name}").is_empty());
    }

    #[test]
    fn tolerates_broken_templates_without_panicking() {
        let s = subs();
        assert_eq!(s.apply("${unterminated"), "${unterminated");
        assert_eq!(s.apply("$"), "$");
        assert_eq!(s.apply("${}"), "${}");
        // multi-byte input must not be split mid-character
        assert_eq!(s.apply("中文${version_name}路径"), "中文1.20.4路径");
        assert!(s.unresolved("${unterminated").is_empty());
    }

    #[test]
    fn flattens_plain_and_rule_gated_entries() {
        let json = serde_json::json!([
            "--username",
            "${auth_player_name}",
            { "rules": [{ "action": "allow", "features": { "is_demo_user": true } }],
              "value": "--demo" },
            { "rules": [{ "action": "allow", "features": { "has_custom_resolution": true } }],
              "value": ["--width", "${resolution_width}"] },
            { "rules": [{ "action": "allow", "os": { "name": "osx" } }],
              "value": "-XstartOnFirstThread" }
        ]);
        let values = json.as_array().unwrap();

        let mut features = Features::new();
        features.insert("has_custom_resolution".into(), true);
        let out = flatten_arguments(values, &android(), &features).unwrap();
        assert_eq!(
            out,
            vec![
                "--username",
                "${auth_player_name}",
                "--width",
                "${resolution_width}"
            ]
        );

        // demo enabled => `--demo` appears; macOS-only entry never does
        features.insert("is_demo_user".into(), true);
        let out = flatten_arguments(values, &android(), &features).unwrap();
        assert!(out.contains(&"--demo".to_string()));
        assert!(!out.contains(&"-XstartOnFirstThread".to_string()));
    }

    #[test]
    fn malformed_entries_are_reported() {
        let json = serde_json::json!([42]);
        let err =
            flatten_arguments(json.as_array().unwrap(), &android(), &Features::new()).unwrap_err();
        assert!(
            err.to_string().contains("malformed argument entry"),
            "{err}"
        );

        let json = serde_json::json!([{ "value": { "nope": 1 } }]);
        assert!(flatten_arguments(json.as_array().unwrap(), &android(), &Features::new()).is_err());

        let json = serde_json::json!([{ "value": [1] }]);
        assert!(flatten_arguments(json.as_array().unwrap(), &android(), &Features::new()).is_err());

        let json = serde_json::json!([{ "rules": "not-a-list", "value": "x" }]);
        assert!(flatten_arguments(json.as_array().unwrap(), &android(), &Features::new()).is_err());
    }

    #[test]
    fn legacy_arguments_split_on_whitespace() {
        let a = split_legacy_arguments("--username ${auth_player_name}  --version ${version_name}");
        assert_eq!(a.len(), 4);
        assert!(split_legacy_arguments("   ").is_empty());
    }

    #[test]
    fn rules_default_to_deny_when_present() {
        let allow: Vec<Rule> =
            serde_json::from_value(serde_json::json!([{ "action": "allow" }])).unwrap();
        let deny: Vec<Rule> = serde_json::from_value(
            serde_json::json!([{ "action": "allow" }, { "action": "disallow" }]),
        )
        .unwrap();
        let os_only: Vec<Rule> = serde_json::from_value(
            serde_json::json!([{ "action": "allow", "os": { "name": "windows" } }]),
        )
        .unwrap();
        assert!(rules_allow(&[], &android(), &Features::new()));
        assert!(rules_allow(&allow, &android(), &Features::new()));
        assert!(!rules_allow(&deny, &android(), &Features::new()));
        assert!(!rules_allow(&os_only, &android(), &Features::new()));
    }

    #[test]
    fn prunes_unresolved_values_together_with_their_flag() {
        let s = subs();
        let args: Vec<String> = [
            "--username",
            "${auth_player_name}",
            "--xuid",
            "${auth_xuid}",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let pruned = prune_unresolved(&args, &s);
        assert_eq!(pruned.args, vec!["--username", "Steve"]);
        assert_eq!(pruned.dropped.len(), 1);
        assert!(pruned.dropped[0].contains("--xuid"), "{:?}", pruned.dropped);
        assert!(pruned.dropped[0].contains("auth_xuid"));
    }

    #[test]
    fn prunes_partially_resolved_values() {
        let s = subs();
        let args = vec!["--session".to_string(), "token:${auth_session}".to_string()];
        let pruned = prune_unresolved(&args, &s);
        assert!(pruned.args.is_empty());
        assert_eq!(pruned.dropped.len(), 1);
    }

    #[test]
    fn flag_lookup() {
        let args: Vec<String> = vec!["--width".into(), "800".into(), "-Xmx1024M".into()];
        assert!(has_flag(&args, "--width"));
        assert!(!has_flag(&args, "--height"));
        assert!(has_flag(&["-Dfoo=bar".to_string()], "-Dfoo"));
    }
}
