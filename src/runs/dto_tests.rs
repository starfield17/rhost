use super::dto::bind;
use serde_json::json;
#[test]
fn same_kind_foreign_evidence_is_refused() {
    let expected = "a".repeat(32);
    let other = "c".repeat(32);
    let key = format!("2000000000-{}", "b".repeat(64));
    let foreign = format!("2000000000-{}", "d".repeat(64));
    let good = json!({"run":{"generation":expected,"run_id":key,"status":"exited","completion":{"exit_code":0}}});
    assert!(bind(2, &good, Some(&expected), Some(&key)).is_ok());
    assert!(bind(3, &good, Some(&expected), Some(&key)).is_ok());
    assert!(bind(2, &good, Some(&other), Some(&key)).is_err());
    assert!(bind(3, &good, Some(&expected), Some(&foreign)).is_err());
    assert!(bind(4, &json!([good["run"]]), Some(&other), None).is_err());
    assert!(
        bind(
            5,
            &json!({"run_id":foreign,"delivered":true}),
            Some(&expected),
            Some(&key)
        )
        .is_err()
    );
}
