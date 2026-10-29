//! Conservative OpenSSH pre-submission evidence shared by both transports.
use crate::domain::PreExecFailure;
pub(crate) fn pre_execution(stderr: &str, exit: Option<i32>) -> Option<PreExecFailure> {
    let lower = stderr.to_lowercase();
    if exit == Some(255)
        && lower.lines().any(|line| {
            (line.starts_with("ssh: connect to host ")
                || line.starts_with("ssh: connect to address "))
                && line.contains(" port ")
                && line.ends_with(": operation not permitted")
        })
    {
        return Some(PreExecFailure::Connection);
    }
    // The wrapper itself could not start. Quote the offending line's meaning by
    // name rather than the first stderr line, which is often an unrelated
    // banner when ssh runs at a higher log level.
    if lower.contains("command not found")
        && (lower.contains("setsid") || lower.contains("bash") || lower.contains("base64"))
    {
        return Some(PreExecFailure::DependencyMissing);
    }
    if lower.contains("controlpath too long")
        || lower.contains("unix_listener: cannot bind to path")
    {
        return Some(PreExecFailure::Config);
    }
    if lower.contains("host key verification failed") {
        return Some(PreExecFailure::HostKey);
    }
    if exit == Some(255)
        && (lower.contains("permission denied (publickey")
            || lower.contains("permission denied (password")
            || lower.contains("permission denied (keyboard-interactive")
            || lower.contains("no supported authentication methods"))
    {
        return Some(PreExecFailure::Authentication);
    }
    if lower.contains("could not resolve hostname") || lower.contains("name or service not known") {
        return Some(PreExecFailure::HostUnknown);
    }
    if lower.contains("connection refused")
        || lower.contains("connection timed out")
        || lower.contains("operation timed out")
        || lower.contains("no route to host")
    {
        return Some(PreExecFailure::Connection);
    }
    // A zero status with no marker, and an unexplained non-zero one, are both
    // execution uncertainty: never a known success, and never an assumed
    // connectivity failure that a caller may blindly retry (EXEC-005).
    None
}
