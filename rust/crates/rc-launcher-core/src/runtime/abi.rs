//! Android ABIs supported by the prebuilt FCL JRE packages (task 6).
//!
//! FCL ships one `bin-<suffix>.tar.xz` per ABI plus a shared `universal.tar.xz`.
//! The [`Abi`] enum models the four ABIs we provision and knows how to map
//! itself to both the Android ABI string (`arm64-v8a`, …) and FCL's archive
//! file suffix (`arm64`, `arm`, `x86`, `x86_64`).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A CPU architecture / Android ABI we can install a JRE for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Abi {
    /// `arm64-v8a` — 64-bit ARM (the only ABI present in the prebuilt APK).
    Arm64V8a,
    /// `armeabi-v7a` — 32-bit ARM.
    ArmeabiV7a,
    /// `x86` — 32-bit Intel.
    X86,
    /// `x86_64` — 64-bit Intel.
    X86_64,
}

impl Abi {
    /// All ABIs the runtime layer knows how to provision.
    pub fn all() -> &'static [Abi] {
        &[Abi::Arm64V8a, Abi::ArmeabiV7a, Abi::X86, Abi::X86_64]
    }

    /// The Android NDK ABI triple string (used for directory / naming).
    pub fn as_android_abi(self) -> &'static str {
        match self {
            Abi::Arm64V8a => "arm64-v8a",
            Abi::ArmeabiV7a => "armeabi-v7a",
            Abi::X86 => "x86",
            Abi::X86_64 => "x86_64",
        }
    }

    /// FCL's `bin-<suffix>.tar.xz` file suffix for this ABI.
    pub fn as_fcl_suffix(self) -> &'static str {
        match self {
            Abi::Arm64V8a => "arm64",
            Abi::ArmeabiV7a => "arm",
            Abi::X86 => "x86",
            Abi::X86_64 => "x86_64",
        }
    }

    /// Parse an Android ABI string (`arm64-v8a`, …) into an [`Abi`].
    pub fn from_android_abi(s: &str) -> Option<Abi> {
        match s {
            "arm64-v8a" => Some(Abi::Arm64V8a),
            "armeabi-v7a" => Some(Abi::ArmeabiV7a),
            "x86" => Some(Abi::X86),
            "x86_64" => Some(Abi::X86_64),
            _ => None,
        }
    }

    /// The archive file name for this ABI's per-ABI JRE slice.
    pub fn bin_archive_name(self) -> String {
        format!("bin-{}.tar.xz", self.as_fcl_suffix())
    }
}

impl fmt::Display for Abi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_android_abi())
    }
}

impl Serialize for Abi {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_android_abi())
    }
}

impl<'de> Deserialize<'de> for Abi {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Abi::from_android_abi(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown ABI: {s}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_abi_roundtrip() {
        for abi in Abi::all() {
            let s = abi.as_android_abi();
            assert_eq!(Abi::from_android_abi(s), Some(*abi));
        }
    }

    #[test]
    fn fcl_suffix_matches_apk_layout() {
        assert_eq!(Abi::Arm64V8a.bin_archive_name(), "bin-arm64.tar.xz");
        assert_eq!(Abi::ArmeabiV7a.bin_archive_name(), "bin-arm.tar.xz");
        assert_eq!(Abi::X86.bin_archive_name(), "bin-x86.tar.xz");
        assert_eq!(Abi::X86_64.bin_archive_name(), "bin-x86_64.tar.xz");
    }

    #[test]
    fn serde_as_android_abi() {
        let json = serde_json::to_string(&Abi::Arm64V8a).unwrap();
        assert_eq!(json, "\"arm64-v8a\"");
        let back: Abi = serde_json::from_str("\"armeabi-v7a\"").unwrap();
        assert_eq!(back, Abi::ArmeabiV7a);
    }
}
