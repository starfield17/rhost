use super::DomainError;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootId(String);
impl BootId {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.len() != 36
            || !value.bytes().enumerate().all(|(i, c)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    c == b'-'
                } else {
                    c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
                }
            })
        {
            return Err(DomainError("boot ID must be a lowercase UUID"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
