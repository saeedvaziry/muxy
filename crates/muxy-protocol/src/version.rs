use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Version(pub u16);

pub const V1: Version = Version(1);
// The development schema stays on V1 until the first official release.
pub const SUPPORTED: &[Version] = &[V1];
