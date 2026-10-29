//! Observe OpenSSH control evidence without creating a connection.
use crate::remote::{Error, Observation};
use crate::transport::{Client, ConnectionStatus};
pub(crate) fn observe(
    client: &Client,
    host: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<ConnectionStatus, Error> {
    let observation = Observation::new(client, cancelled);
    observation.check()?;
    let status = client.connection_status_observed(host, cancelled);
    observation.check()?;
    Ok(status)
}
