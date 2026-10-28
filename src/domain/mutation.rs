//! Evidence about requested file bytes, independent of transport availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationEvidence {
    NotApplied,
    Applied,
    Partial,
    Unknown,
}
impl MutationEvidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotApplied => "not_applied",
            Self::Applied => "applied",
            Self::Partial => "partial",
            Self::Unknown => "unknown",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "not_applied" => Some(Self::NotApplied),
            "applied" => Some(Self::Applied),
            "partial" => Some(Self::Partial),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}
