use super::{DomainError, RemotePath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256(String);
impl Sha256 {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(DomainError("SHA-256 must contain 64 lowercase hex digits"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Choosing Create never authorizes replacement. The file adapter must check
/// nonexistence under the same lock used for atomic replacement.
/// ```compile_fail
/// use rhost::domain::{FileWrite, RemotePath};
/// let path = RemotePath::new("~/work/file".into())?;
/// let edit = FileWrite::Replace { path };
/// # Ok::<(), rhost::domain::DomainError>(())
/// ```
#[derive(Debug)]
pub enum FileWrite {
    Create { path: RemotePath },
    Replace { path: RemotePath, expected: Sha256 },
    Patch { path: RemotePath, expected: Sha256 },
}
