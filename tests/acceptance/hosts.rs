//! Stand-ins for the two external things a file operation talks to.
//!
//! `LOCAL_HOST` is what `sshd` would do with the one command argument it is
//! given — run it — so the wrapper's own protocol travels through untouched. The
//! transfer stubs record their argv and then do exactly what a case says.

use crate::support::Harness;

/// A stand-in for `sshd`: run the one command argument it would hand to the
/// account's login shell.
pub(crate) const LOCAL_HOST: &str =
    "for argument in \"$@\"; do remote=\"$argument\"; done\nexec /bin/bash -lc \"$remote\"";

/// The wrapper asks for a session of its own; this host already gives it one.
pub(crate) const LOCAL_SETSID: &str = "exec \"$@\"";

/// Installs the local-host stand-ins, so the candidate's real exec path runs
/// locally without a network.
pub(crate) fn local_host(harness: &Harness) -> Result<(), String> {
    harness.stub("ssh", LOCAL_HOST)?;
    harness.stub("setsid", LOCAL_SETSID)?;
    Ok(())
}

/// An scp stand-in that records its argv (the harness does that) and writes the
/// bytes the JSON is expected to describe. It deliberately does not parse
/// rhost's own protocol: scp has none.
pub(crate) fn scp_stub(body: &str) -> String {
    format!(
        r#"dest=""
for argument in "$@"; do dest="$argument"; done
src=""
index=0
for argument in "$@"; do
  index=$((index + 1))
  if [ "$index" -eq $(($# - 1)) ]; then src="$argument"; fi
done
base=${{src##*:}}
base=${{base##*/}}
if [ -d "$dest" ]; then dest="$dest/$base"; fi
{body}
"#
    )
}

/// A stub rsync that prints the itemized plan the real one prints, plus one line
/// of noise, so the parser has to tell them apart.
pub(crate) fn rsync_stub(extra: &str) -> String {
    format!(
        r#"printf '%s\n' 'RHOSTSYNC|>f+++++++++|a.txt'
printf '%s\n' 'RHOSTSYNC|cd+++++++++|sub/'
printf '%s\n' 'RHOSTSYNC|>f.st......|sub/b.txt'
printf '%s\n' 'RHOSTSYNC|*deleting|orphan.txt'
printf '%s\n' 'rsync: warning: something a human may want'
{extra}
"#
    )
}
