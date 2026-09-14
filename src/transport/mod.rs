//! Remote capabilities: the OpenSSH transport and its execution protocol.
//!
//! rhost orchestrates the system OpenSSH client; it never reimplements SSH,
//! authentication or host-key policy (AGENTS.md §5).
pub mod openssh;
pub mod process;
pub mod protocol;
