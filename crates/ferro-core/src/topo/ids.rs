use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Ids are map keys; inside internally tagged enums serde buffers map keys as
/// strings, so ids accept both numbers and numeric strings.
pub(crate) fn de_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    struct V;
    impl serde::de::Visitor<'_> for V {
        type Value = u64;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an id (integer or numeric string)")
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<u64, E> {
            u64::try_from(v).map_err(E::custom)
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u64, E> {
            v.parse().map_err(E::custom)
        }
    }
    d.deserialize_any(V)
}

macro_rules! topo_id {
    ($(#[$m:meta])* $name:ident, $prefix:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let v = de_u64(d)?;
                u32::try_from(v).map($name).map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

topo_id!(
    /// Persistent vertex id, unique within one body.
    VertexId, "v"
);
topo_id!(
    /// Persistent edge id (a pair of twin half-edges).
    EdgeId, "e"
);
topo_id!(
    /// Directed use of an edge by one loop. Internal; may change on rebuild.
    HalfEdgeId, "h"
);
topo_id!(
    /// A closed cycle of half-edges bounding a face.
    LoopId, "l"
);
topo_id!(
    /// Persistent face id, unique within one body.
    FaceId, "f"
);
