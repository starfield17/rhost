//! Request cancellation is checked before a local subprocess can have effects.
use rhost::transport::{Recorder, Spec, StdinSource, run};
#[test]
fn already_cancelled_observation_never_spawns_its_program() {
    let mut out = Recorder::new(4096);
    let mut err = Recorder::new(4096);
    let result = run(
        &Spec {
            program: "/rhost-observation-must-not-spawn",
            args: &[],
            stdin: StdinSource::Closed,
            deadline: None,
            cancelled: &|| true,
            group: false,
        },
        &mut out,
        &mut err,
    );
    assert!(result.cancelled);
    assert_eq!(result.exit, None);
    assert!(result.failure.is_none());
    assert!(out.body().is_empty());
}
