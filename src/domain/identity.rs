use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainError(pub &'static str);
impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for DomainError {}

/// A remote completion status, never a sentinel for missing evidence.
/// ```compile_fail
/// let status = rhost::domain::ExitCode(-1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(u8);
impl ExitCode {
    pub fn new(value: i32) -> Result<Self, DomainError> {
        u8::try_from(value)
            .map(Self)
            .map_err(|_| DomainError("exit code must be between 0 and 255"))
    }
    pub fn get(self) -> u8 {
        self.0
    }
}

/// Canonical resolved identity. A caller reference cannot be substituted.
/// ```compile_fail
/// use rhost::domain::{SessionId, SessionRef};
/// fn close(_: SessionId) {}
/// let reference = SessionRef::new("example".into())?;
/// close(reference);
/// # Ok::<(), rhost::domain::DomainError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(String);
impl SessionId {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.is_empty()
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(DomainError("invalid canonical session identity"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef(String);
impl SessionRef {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(DomainError("invalid session reference"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePath(String);
impl RemotePath {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.is_empty() || value.contains('\0') {
            return Err(DomainError(
                "remote path must be nonempty and contain no NUL",
            ));
        }
        if value.len() >= 2
            && ((value.starts_with('\'') && value.ends_with('\''))
                || (value.starts_with('"') && value.ends_with('"')))
        {
            return Err(DomainError("outer quotes are not part of a remote path"));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validates a transport-generated nonce; it does not generate randomness or
/// by itself prove that a completion message belongs to an invocation.
#[derive(Debug, PartialEq, Eq)]
pub struct InvocationToken(String);
impl InvocationToken {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(DomainError(
                "invocation token must contain 32 lowercase hex digits",
            ));
        }
        Ok(Self(value))
    }
    pub fn matches(&self, observed: &str) -> bool {
        self.0 == observed
    }
}

/// Completion evidence can only be constructed by matching the invocation token.
/// Syntax validation and matching do not replace secure token generation in the
/// transport, nor do they establish anything about detached descendants.
/// ```compile_fail
/// let evidence = rhost::domain::VerifiedCompletion { exit_code: rhost::domain::ExitCode::new(0)? };
/// # Ok::<(), rhost::domain::DomainError>(())
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedCompletion {
    exit_code: ExitCode,
}
impl VerifiedCompletion {
    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }
}
impl InvocationToken {
    /// Consumes the expected token; evidence must not be replayed into another call.
    /// ```compile_fail
    /// use rhost::domain::{InvocationToken, ExitCode};
    /// let token = InvocationToken::new("a".repeat(32))?;
    /// let first = token.verify_completion(&"a".repeat(32), ExitCode::new(0)?)?;
    /// let replay = token.verify_completion(&"a".repeat(32), ExitCode::new(0)?)?;
    /// # Ok::<(), rhost::domain::DomainError>(())
    /// ```
    pub fn verify_completion(
        self,
        observed: &str,
        exit_code: ExitCode,
    ) -> Result<VerifiedCompletion, DomainError> {
        if !self.matches(observed) {
            return Err(DomainError("completion belongs to another invocation"));
        }
        Ok(VerifiedCompletion { exit_code })
    }
}
