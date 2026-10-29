//! Local discovery belongs to the capability, independently of its delivery.
pub(crate) fn observe() -> crate::host::Discovery {
    crate::host::aliases()
}
