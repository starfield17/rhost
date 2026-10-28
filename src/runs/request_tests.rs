fn prepare(raw: &str) -> Result<super::request::Prepared, crate::remote::Error> {
    super::request::prepare("run.start", "gpu", raw)
}
use serde_json::json;
fn request() -> String {
    json!({
        "generation":"a".repeat(32),"key":format!("2000000000-{}","b".repeat(64)),"expires_unix":2000000000u64,
        "cwd":"/tmp","program":{"argv":["/bin/true"]},"environment":{"inherit":false,"values":{}}
    }).to_string()
}
#[test]
fn request_identity_and_closed_fields_are_validated_before_transport() {
    assert!(prepare(&request()).is_ok());
    let duplicate = request().replace("\"values\":{}", "\"values\":{\"A\":\"1\",\"A\":\"2\"}");
    assert!(prepare(&duplicate).is_err());
    for (name, value) in [
        ("expires_unix", json!(2000000001u64)),
        ("deadline_ms", json!(null)),
        ("stdin_base64", json!("\n")),
        ("unknown", json!(true)),
    ] {
        let mut document: serde_json::Value =
            serde_json::from_str(&request()).unwrap_or_else(|e| panic!("fixture: {e}"));
        document[name] = value;
        assert!(prepare(&document.to_string()).is_err(), "{name}");
    }
}
