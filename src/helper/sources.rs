//! Embedded sources are the only source-build authority, never remote packages.
use sha2::{Digest, Sha256};
pub const SOURCES: &[(&str, &str)] = &[
    ("install.c", include_str!("../../helper/install.c")),
    ("main.c", include_str!("../../helper/main.c")),
    ("monitor.c", include_str!("../../helper/monitor.c")),
    ("protocol.c", include_str!("../../helper/protocol.c")),
    ("protocol.h", include_str!("../../helper/protocol.h")),
    ("run.c", include_str!("../../helper/run.c")),
    ("sha256.c", include_str!("../../helper/sha256.c")),
    ("start.c", include_str!("../../helper/start.c")),
    ("state.c", include_str!("../../helper/state.c")),
    ("state.h", include_str!("../../helper/state.h")),
];
pub fn digest() -> String {
    let mut hash = Sha256::new();
    for (name, source) in SOURCES {
        hash.update(format!("helper/{name}\0"));
        hash.update(source);
        hash.update([0]);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
pub fn script(cache: &str, marker: &str) -> String {
    let digest = digest();
    let mut script = String::from(
        "set -eu\numask 077\ntmp=$(mktemp -d)\ntrap 'rm -rf -- \"$tmp\"' EXIT HUP INT TERM\n",
    );
    script.push_str("command -v cc >/dev/null 2>&1 || { printf '%s\\n' 'remote cc is unavailable' >&2; exit 127; }\n");
    for (name, source) in SOURCES {
        let delimiter = format!("RHOST_SOURCE_{digest}");
        script.push_str(&format!(
            "cat > \"$tmp/{name}\" <<'{delimiter}'\n{source}{delimiter}\n"
        ));
    }
    script.push_str(&format!("cc -std=c99 -O2 '-DRHOST_SOURCE_DIGEST=\"{digest}\"' \"$tmp/\"*.c -o \"$tmp/rhost-helper\"\n"));
    script.push_str(&format!(
        "\"$tmp/rhost-helper\" --install-cache {} {digest}\nprintf '%s\\n' {}\n",
        crate::shell::path_quote(cache),
        crate::shell::quote(marker)
    ));
    script
}
pub fn binary_script(cache: &str, marker: &str) -> String {
    format!(
        "set -eu\numask 077\ntmp=$(mktemp -d)\ntrap 'rm -rf -- \"$tmp\"' EXIT HUP INT TERM\ncat > \"$tmp/rhost-helper\"\nchmod 700 \"$tmp/rhost-helper\"\n\"$tmp/rhost-helper\" --install-cache {} {}\nprintf '%s\\n' {}\n",
        crate::shell::path_quote(cache),
        digest(),
        crate::shell::quote(marker)
    )
}
