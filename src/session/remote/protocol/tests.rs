use super::*;

fn meta_line(id: &str, alive: &str, name: &str) -> String {
    let meta = format!(
        r#"{{"schema_version":1,"id":"{id}","name":"{name}","tmux_session":"rhost_s_{id}","created_at":"now","shell":"bash"}}"#
    );
    format!(
        "RHOST_META\t{id}\t{alive}\t{}\t\n",
        base64::encode(meta.as_bytes())
    )
}

#[test]
fn list_keeps_records_sorts_them_and_reports_a_helper_refusal() {
    let stdout = format!(
        "noise\n{}{}{}",
        meta_line("s_b", "no", "beta"),
        meta_line("s_a", "yes", "alpha"),
        "RHOST_META\ts_bad\tyes\tnot-base64\n"
    );
    let rows = parse_list(&stdout).unwrap_or_else(|failure| panic!("{failure:?}"));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "s_a");
    assert!(rows[0].alive && rows[0].meta.name == "alpha");
    assert_eq!(rows[1].id, "s_b");
    assert!(!rows[1].alive);
    assert_eq!(
        parse_list("RHOST_ERR=notmux\n").err(),
        Some(HelperFailure::NoTmux)
    );
}

#[test]
fn a_resolved_cwd_sidecar_is_read_and_a_legacy_record_falls_back() {
    // A record this build wrote carries the resolved directory as the fourth
    // field.
    let meta = r#"{"schema_version":1,"id":"s_new","name":"new","tmux_session":"rhost_s_s_new","created_at":"now","shell":"bash","initial_cwd":"~"}"#;
    let line = format!(
        "RHOST_META\ts_new\tyes\t{}\t{}\n",
        base64::encode(meta.as_bytes()),
        base64::encode(b"/home/dev"),
    );
    let rows = parse_list(&line).unwrap_or_else(|failure| panic!("{failure:?}"));
    assert_eq!(rows[0].resolved_cwd.as_deref(), Some("/home/dev"));

    // A three-field record from an older build has no sidecar, so the reader
    // reports none rather than inventing one; the metadata literal remains.
    let legacy = format!(
        "RHOST_META\ts_old\tyes\t{}\n",
        base64::encode(legacy_meta("s_old").as_bytes())
    );
    let rows = parse_list(&legacy).unwrap_or_else(|failure| panic!("{failure:?}"));
    assert_eq!(rows[0].resolved_cwd, None);
    assert_eq!(rows[0].meta.initial_cwd, "~");
}

fn legacy_meta(id: &str) -> String {
    format!(
        r#"{{"schema_version":1,"id":"{id}","name":"old","tmux_session":"rhost_s_{id}","created_at":"now","shell":"bash","initial_cwd":"~"}}"#
    )
}

fn token() -> String {
    "a".repeat(32)
}

#[test]
fn an_exec_result_needs_this_invocations_token_and_one_status() {
    let good = format!(
        "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=7\nRHOST_OUTPUT={}\n",
        token(),
        base64::encode(b"out\n")
    );
    match parse_exec(&good, &token()) {
        ExecResult::Completed {
            id,
            output,
            exit_code,
        } => {
            assert_eq!(id, "s_ab");
            assert_eq!(output, b"out\n");
            assert_eq!(exit_code, 7);
        }
        other => panic!("{other:?}"),
    }
    for broken in [
        // Another invocation's token, a duplicated status, a status written
        // differently, a missing identity, a stray line, a duplicate identity.
        format!(
            "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\n",
            "b".repeat(32)
        ),
        format!(
            "RHOST_ID=s_ab\nRHOST_TOKEN={0}\nRHOST_EXIT=0\nRHOST_EXIT=1\nRHOST_OUTPUT=\n",
            token()
        ),
        format!(
            "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=07\nRHOST_OUTPUT=\n",
            token()
        ),
        format!("RHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\n", token()),
        format!(
            "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\nnoise\n",
            token()
        ),
        format!(
            "RHOST_ID=s_ab\nRHOST_ID=s_cd\nRHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\n",
            token()
        ),
    ] {
        assert!(
            matches!(
                parse_exec(&broken, &token()),
                ExecResult::Failed {
                    failure: HelperFailure::Protocol,
                    ..
                }
            ),
            "{broken}"
        );
    }
}

#[test]
fn a_refusal_carries_what_owns_the_pane_and_whether_the_shell_came_back() {
    match parse_exec("RHOST_ID=s_ab\nRHOST_FG=cat\nRHOST_ERR=busy\n", &token()) {
        ExecResult::Failed {
            failure,
            id,
            foreground,
            recovered,
        } => {
            assert_eq!(failure, HelperFailure::Busy);
            assert_eq!(id.as_deref(), Some("s_ab"));
            assert_eq!(foreground.as_deref(), Some("cat"));
            assert!(!recovered);
        }
        other => panic!("{other:?}"),
    }
    match parse_exec(
        "RHOST_ID=s_ab\nRHOST_RECOVERED=1\nRHOST_ERR=timeout\n",
        &token(),
    ) {
        ExecResult::Failed {
            failure, recovered, ..
        } => {
            assert_eq!(failure, HelperFailure::Timeout);
            assert!(recovered);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_read_needs_consistent_cursors_and_decodes_its_page() {
    let ok = format!(
        "RHOST_ID=s_ab\nRHOST_FROM=4\nRHOST_NEXT=9\nRHOST_SIZE=20\n{}\n",
        base64::encode(b"hello")
    );
    let read = parse_read(&ok).unwrap_or_else(|failure| panic!("{failure:?}"));
    assert_eq!((read.from, read.next, read.size), (4, 9, 20));
    assert_eq!(read.data, b"hello");
    for broken in [
        // A cursor that goes backwards, a size smaller than the cursor, a
        // missing field, a duplicated one, an unknown field, no identity.
        "RHOST_ID=s_ab\nRHOST_FROM=9\nRHOST_NEXT=4\nRHOST_SIZE=20\n\n",
        "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_NEXT=9\nRHOST_SIZE=4\n\n",
        "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_SIZE=4\n\n",
        "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_FROM=0\nRHOST_NEXT=0\nRHOST_SIZE=0\n\n",
        "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_NEXT=0\nRHOST_SIZE=0\nRHOST_WHAT=1\n\n",
        "RHOST_FROM=0\nRHOST_NEXT=0\nRHOST_SIZE=0\n\n",
        "RHOST_ID=s_ab\nRHOST_FROM=4\nRHOST_NEXT=10\nRHOST_SIZE=20\naGVsbG8=\n",
        "RHOST_ID=s_ab\nRHOST_FROM=4\nRHOST_NEXT=8\nRHOST_SIZE=20\naGVsbG8=\n",
    ] {
        assert_eq!(
            parse_read(broken).err(),
            Some(HelperFailure::Protocol),
            "{broken}"
        );
    }
    assert_eq!(
        parse_read("RHOST_ERR=nosession\n").err(),
        Some(HelperFailure::NoSession)
    );
}
