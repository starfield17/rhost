//! Published v5.2.0 wire evidence remains immutable during schema-5 work.
use sha2::{Digest, Sha256};
#[test]
fn published_schema4_evidence_is_immutable() {
    for (name, bytes, expected) in [
        (
            "schemas/request-v4.schema.json",
            include_bytes!("../schemas/request-v4.schema.json").as_slice(),
            "8d2393a1a2e1ecfc9ab1ae642138684d20bc8d894573a11d0320c204ba061a68",
        ),
        (
            "schemas/result-v4.schema.json",
            include_bytes!("../schemas/result-v4.schema.json").as_slice(),
            "7444f8e30c5b05ae822accb45e94e936159219f95e11ce12b580fd264574518f",
        ),
        (
            "tests/fixtures/result-v4-valid.json",
            include_bytes!("../tests/fixtures/result-v4-valid.json").as_slice(),
            "73756c70ef1cb19fa36503bff0fe57a1b059c8e5ea933b92a6ecb5706769f383",
        ),
    ] {
        let actual = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(actual, expected, "published evidence changed: {name}");
    }
}
