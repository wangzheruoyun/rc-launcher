//! Dependency & conflict resolution (task 8).
//!
//! Given the set of *enabled* mods in an instance and the instance's Minecraft
//! version, [`resolve_issues`] walks every mod's `depends` / `breaks` /
//! `conflicts` edges (produced by `metadata`) and reports concrete problems as
//! [`ModIssue`]s. This mirrors the validation FCL's `ModManager` performs
//! before launch, so a broken loadout is surfaced to the user instead of
//! crashing the game.

use crate::mods::metadata::ModMetadata;
use std::collections::HashMap;

/// What kind of problem an issue describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModIssueKind {
    /// A hard dependency (`depends` / `requiredMods`) is not present.
    MissingDependency,
    /// A hard dependency is present but its version is out of range.
    IncompatibleDependency,
    /// Two mods declare a `breaks` / `conflicts` edge against each other.
    Conflict,
    /// The mod requires a Minecraft version we are not running.
    IncompatibleMinecraft,
    /// Two enabled mods share the same mod id (version clash).
    DuplicateMod,
}

impl ModIssueKind {
    /// Short, stable code used by the FFI / UI layer.
    pub fn code(self) -> &'static str {
        match self {
            ModIssueKind::MissingDependency => "missing_dependency",
            ModIssueKind::IncompatibleDependency => "incompatible_dependency",
            ModIssueKind::Conflict => "conflict",
            ModIssueKind::IncompatibleMinecraft => "incompatible_minecraft",
            ModIssueKind::DuplicateMod => "duplicate_mod",
        }
    }
}

/// A concrete problem found while validating a mod loadout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModIssue {
    /// Mod id that *triggered* the issue (the one declaring the edge).
    pub source_modid: String,
    /// Kind of problem.
    pub kind: ModIssueKind,
    /// The other mod id (dependency / conflict target), if applicable.
    pub target: Option<String>,
    /// Human-readable explanation (zh/en friendly, no PII).
    pub detail: String,
}

impl ModIssue {
    fn new(
        source_modid: impl Into<String>,
        kind: ModIssueKind,
        target: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            source_modid: source_modid.into(),
            kind,
            target,
            detail: detail.into(),
        }
    }
}

/// Minimal view the resolver needs from a mod record.
pub trait ModView {
    /// The parsed metadata (ids, deps, conflicts, mc constraint).
    fn meta(&self) -> &ModMetadata;
    /// Whether the mod is currently enabled.
    fn is_enabled(&self) -> bool;
}

impl ModView for ModMetadata {
    fn meta(&self) -> &ModMetadata {
        self
    }
    fn is_enabled(&self) -> bool {
        true
    }
}

/// Resolve every issue in `mods` (any type implementing [`ModView`]) against
/// the instance's `game_version`.
///
/// Disabled mods are ignored entirely (they are not loaded, so they neither
/// satisfy dependencies nor participate in conflicts). `game_version` may be
/// `None` (e.g. before a version is chosen); in that case Minecraft-version
/// edges are skipped rather than reported.
pub fn resolve_issues<M: ModView>(mods: &[M], game_version: Option<&str>) -> Vec<ModIssue> {
    let mut issues = Vec::new();

    // Index enabled mods by id → their versions.
    let mut by_id: HashMap<String, Vec<Option<String>>> = HashMap::new();
    for m in mods {
        if !m.is_enabled() {
            continue;
        }
        by_id
            .entry(m.meta().modid.clone())
            .or_default()
            .push(m.meta().version.clone());
    }

    for m in mods {
        if !m.is_enabled() {
            continue;
        }
        let meta = m.meta();

        // --- Minecraft version compatibility ---
        if let (Some(constraint), Some(gv)) = (&meta.minecraft, game_version) {
            if !constraint.matches(gv) {
                issues.push(ModIssue::new(
                    &meta.modid,
                    ModIssueKind::IncompatibleMinecraft,
                    Some("minecraft".to_string()),
                    format!("requires Minecraft {}, running {}", constraint.raw(), gv),
                ));
            }
        }

        // --- Dependencies ---
        for dep in &meta.dependencies {
            if dep.modid == "minecraft" {
                // Already covered above; skip to avoid duplicate reports.
                continue;
            }
            let present = by_id.get(&dep.modid).cloned().unwrap_or_default();
            if present.is_empty() {
                if dep.required {
                    issues.push(ModIssue::new(
                        &meta.modid,
                        ModIssueKind::MissingDependency,
                        Some(dep.modid.clone()),
                        format!(
                            "requires '{}'{} which is not installed",
                            dep.modid,
                            dep.range
                                .as_ref()
                                .map(|r| format!(" ({})", r.raw()))
                                .unwrap_or_default()
                        ),
                    ));
                }
                continue;
            }
            if dep.required {
                let satisfied = present.iter().any(|v| match (v, &dep.range) {
                    (Some(ver), Some(c)) => c.matches(ver),
                    (_, None) => true,
                    (None, Some(_)) => false,
                });
                if !satisfied {
                    issues.push(ModIssue::new(
                        &meta.modid,
                        ModIssueKind::IncompatibleDependency,
                        Some(dep.modid.clone()),
                        format!(
                            "requires '{}'{} but an incompatible version is installed",
                            dep.modid,
                            dep.range
                                .as_ref()
                                .map(|r| format!(" ({})", r.raw()))
                                .unwrap_or_default()
                        ),
                    ));
                }
            }
        }

        // --- Conflicts / breaks ---
        for cf in &meta.conflicts {
            if cf.modid == "minecraft" {
                if let Some(gv) = game_version {
                    if cf.range.as_ref().map(|c| c.matches(gv)).unwrap_or(true) {
                        issues.push(ModIssue::new(
                            &meta.modid,
                            ModIssueKind::IncompatibleMinecraft,
                            Some("minecraft".to_string()),
                            format!(
                                "breaks on Minecraft {} (running {})",
                                cf.range.as_ref().map(|c| c.raw()).unwrap_or("any"),
                                gv
                            ),
                        ));
                    }
                }
                continue;
            }
            let present = by_id.get(&cf.modid).cloned().unwrap_or_default();
            let clashes = present.iter().any(|v| match (v, &cf.range) {
                (Some(ver), Some(c)) => c.matches(ver),
                (_, None) => true,
                (None, Some(_)) => false,
            });
            if !present.is_empty() && clashes {
                issues.push(ModIssue::new(
                    &meta.modid,
                    ModIssueKind::Conflict,
                    Some(cf.modid.clone()),
                    format!(
                        "conflicts with '{}'{}",
                        cf.modid,
                        cf.range
                            .as_ref()
                            .map(|r| format!(" ({})", r.raw()))
                            .unwrap_or_default()
                    ),
                ));
            }
        }
    }

    // --- Duplicate ids among enabled mods ---
    for m in mods {
        if !m.is_enabled() {
            continue;
        }
        let meta = m.meta();
        let enabled_same_id = mods
            .iter()
            .filter(|x| x.is_enabled() && x.meta().modid == meta.modid)
            .count();
        if enabled_same_id > 1 {
            issues.push(ModIssue::new(
                &meta.modid,
                ModIssueKind::DuplicateMod,
                Some(meta.modid.clone()),
                format!(
                    "{} enabled mods share the id '{}'",
                    enabled_same_id, meta.modid
                ),
            ));
            // Avoid N duplicate reports; we already reported enough.
            break;
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::loader::ModLoader;
    use crate::mods::metadata::ModMetadata;

    fn mod_with(id: &str, mc: Option<&str>, deps: Vec<ModMetadata>) -> ModMetadata {
        let _ = deps;
        let mut m = ModMetadata::empty(ModLoader::Fabric);
        m.modid = id.into();
        m.name = id.into();
        if let Some(mc) = mc {
            m.minecraft = crate::mods::constraint::VersionConstraint::parse(mc).ok();
        }
        m
    }

    #[test]
    fn missing_dependency_detected() {
        let a = mod_with("a", Some(">=1.16"), vec![]);
        let mut a = a;
        a.dependencies
            .push(crate::mods::metadata::ModDependency::new("b", None, true));
        let issues = resolve_issues(&[a], Some("1.18.2"));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, ModIssueKind::MissingDependency);
        assert_eq!(issues[0].target.as_deref(), Some("b"));
    }

    #[test]
    fn satisfied_dependency_no_issue() {
        let mut a = mod_with("a", Some(">=1.16"), vec![]);
        a.dependencies
            .push(crate::mods::metadata::ModDependency::new("b", None, true));
        let b = mod_with("b", None, vec![]);
        let issues = resolve_issues(&[a, b], Some("1.18.2"));
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn incompatible_minecraft() {
        let a = mod_with("a", Some(">=1.18"), vec![]);
        let issues = resolve_issues(&[a], Some("1.16.5"));
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, ModIssueKind::IncompatibleMinecraft);
    }

    #[test]
    fn conflict_detected() {
        let mut a = mod_with("a", None, vec![]);
        a.conflicts
            .push(crate::mods::metadata::ModDependency::new("b", None, true));
        let b = mod_with("b", None, vec![]);
        let issues = resolve_issues(&[a, b], Some("1.18.2"));
        assert!(issues.iter().any(|i| i.kind == ModIssueKind::Conflict));
    }

    #[test]
    fn disabled_mods_ignored() {
        use crate::mods::LocalModFile;
        let mut a = mod_with("a", None, vec![]);
        a.conflicts
            .push(crate::mods::metadata::ModDependency::new("b", None, true));
        let b = mod_with("b", None, vec![]);
        // `b` is disabled (file name carries the `.disabled` suffix); it must
        // be ignored by the resolver entirely.
        let a_file = LocalModFile {
            path: std::path::PathBuf::from("/x/a.jar"),
            file_name: "a.jar".to_string(),
            loader: ModLoader::Fabric,
            metadata: vec![a],
        };
        let b_file = LocalModFile {
            path: std::path::PathBuf::from("/x/b.jar.disabled"),
            file_name: "b.jar.disabled".to_string(),
            loader: ModLoader::Fabric,
            metadata: vec![b],
        };
        let issues = resolve_issues(&[a_file, b_file], Some("1.18.2"));
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn duplicate_mod_detected() {
        let a1 = mod_with("dup", None, vec![]);
        let a2 = mod_with("dup", None, vec![]);
        let issues = resolve_issues(&[a1, a2], Some("1.18.2"));
        assert!(issues.iter().any(|i| i.kind == ModIssueKind::DuplicateMod));
    }
}
