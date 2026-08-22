//! Java versions we provision prebuilt JREs for (task 6).
//!
//! FCL packages each JRE under `assets/app_runtime/java/jre<major>/`. The
//! [`JavaVersion`] enum models the versions we ship (8 / 17 / 21, plus 25 which
//! is already present in newer FCL builds) and maps to both the `jre<major>`
//! directory name and the numeric major version.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Java feature release we can install a JRE for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JavaVersion {
    /// Java 8 — required by older Forge / LWJGL2 profiles.
    Java8,
    /// Java 17 — the current baseline for modern Minecraft.
    Java17,
    /// Java 21 — required by the newest Minecraft versions.
    Java21,
    /// Java 25 — shipped by the latest FCL builds.
    Java25,
}

impl JavaVersion {
    /// All Java versions the runtime layer can provision.
    pub fn all() -> &'static [JavaVersion] {
        &[
            JavaVersion::Java8,
            JavaVersion::Java17,
            JavaVersion::Java21,
            JavaVersion::Java25,
        ]
    }

    /// The `jre<major>` directory name used by FCL's assets layout.
    pub fn as_jre_dir(self) -> &'static str {
        match self {
            JavaVersion::Java8 => "jre8",
            JavaVersion::Java17 => "jre17",
            JavaVersion::Java21 => "jre21",
            JavaVersion::Java25 => "jre25",
        }
    }

    /// The numeric major version (8 / 17 / 21 / 25).
    pub fn major(self) -> u32 {
        match self {
            JavaVersion::Java8 => 8,
            JavaVersion::Java17 => 17,
            JavaVersion::Java21 => 21,
            JavaVersion::Java25 => 25,
        }
    }

    /// Parse a `jre<major>` directory name into a [`JavaVersion`].
    pub fn from_jre_dir(s: &str) -> Option<JavaVersion> {
        match s {
            "jre8" => Some(JavaVersion::Java8),
            "jre17" => Some(JavaVersion::Java17),
            "jre21" => Some(JavaVersion::Java21),
            "jre25" => Some(JavaVersion::Java25),
            _ => None,
        }
    }

    /// Parse a numeric major version into a [`JavaVersion`].
    pub fn from_major(m: u32) -> Option<JavaVersion> {
        match m {
            8 => Some(JavaVersion::Java8),
            17 => Some(JavaVersion::Java17),
            21 => Some(JavaVersion::Java21),
            25 => Some(JavaVersion::Java25),
            _ => None,
        }
    }
}

impl fmt::Display for JavaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Java {}", self.major())
    }
}

impl Serialize for JavaVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_jre_dir())
    }
}

impl<'de> Deserialize<'de> for JavaVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        JavaVersion::from_jre_dir(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown Java version: {s}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jre_dir_roundtrip() {
        for v in JavaVersion::all() {
            assert_eq!(JavaVersion::from_jre_dir(v.as_jre_dir()), Some(*v));
            assert_eq!(JavaVersion::from_major(v.major()), Some(*v));
        }
    }

    #[test]
    fn display_and_serde() {
        assert_eq!(JavaVersion::Java17.to_string(), "Java 17");
        let json = serde_json::to_string(&JavaVersion::Java21).unwrap();
        assert_eq!(json, "\"jre21\"");
        let back: JavaVersion = serde_json::from_str("\"jre8\"").unwrap();
        assert_eq!(back, JavaVersion::Java8);
    }
}
