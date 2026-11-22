use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Document-local element id. Never reused within a document (D-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ElementId(pub u64);

impl<'de> Deserialize<'de> for ElementId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::topo::de_u64(d).map(ElementId)
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Stable, globally unique id (UUID, normally v7). Survives save, copy between
/// files, export and the Revit round-trip (D-13). Serialised as the canonical
/// 36-character UUID string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid(pub u128);

impl Uid {
    /// Build a version-7 UUID from a Unix time in milliseconds and random
    /// bits. Pure: callers supply time and randomness, so the kernel stays
    /// deterministic (D-19).
    pub fn v7(unix_ms: u64, random: u128) -> Uid {
        let ts = (unix_ms as u128 & 0xFFFF_FFFF_FFFF) << 80;
        let ver = 0x7u128 << 76;
        let rand_a = (random & 0xFFF) << 64;
        let var = 0b10u128 << 62;
        let rand_b = (random >> 12) & ((1u128 << 62) - 1);
        Uid(ts | ver | rand_a | var | rand_b)
    }

    pub fn version(&self) -> u8 {
        ((self.0 >> 76) & 0xF) as u8
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = format!("{:032x}", self.0);
        write!(
            f,
            "{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    }
}

impl FromStr for Uid {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 || s.len() != 36 {
            return Err("UUID must be 36 characters (8-4-4-4-12)");
        }
        u128::from_str_radix(&hex, 16)
            .map(Uid)
            .map_err(|_| "UUID contains non-hex characters")
    }
}

impl Serialize for Uid {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Uid {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v7_layout_and_string_roundtrip() {
        let u = Uid::v7(1_764_000_000_000, 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        assert_eq!(u.version(), 7);
        let s = u.to_string();
        assert_eq!(s.len(), 36);
        assert_eq!(s.parse::<Uid>().unwrap(), u);
        let json = serde_json::to_string(&u).unwrap();
        assert_eq!(serde_json::from_str::<Uid>(&json).unwrap(), u);
    }

    #[test]
    fn v7_sorts_by_time() {
        assert!(Uid::v7(1000, u128::MAX) < Uid::v7(1001, 0));
    }

    #[test]
    fn bad_uuid_strings_fail() {
        assert!("not-a-uuid".parse::<Uid>().is_err());
        assert!("0123456789abcdef0123456789abcdef".parse::<Uid>().is_err());
        assert!("zzzzzzzz-0000-0000-0000-000000000000"
            .parse::<Uid>()
            .is_err());
    }
}
