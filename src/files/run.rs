//! Running one parsed `fs` operation and delivering its answer.

use super::backend::Tools;
use super::dto;
use super::input::{parse_patch_document, read_local_input};
use super::render;
use super::surface::Fs;
use crate::audit;
use crate::audit::Timer;
use crate::audit::record;
use crate::cli::{Failure, Sink};
use crate::remote::Error;
use crate::transport::Client;
use std::time::Duration;

/// Runs one parsed file operation against the host, delivering either the
/// envelope the caller asked for or the failure both renderings agree on.
pub(crate) fn run(
    sink: &mut Sink,
    client: &Client,
    command: Fs,
    json: bool,
    stop: crate::files::Stop<'_>,
) -> u8 {
    match command {
        Fs::Put {
            host,
            local,
            remote,
            timeout_nanos,
            resume,
            checksum,
            parents,
        } => {
            let timer = Timer::start("fs.put", &host);
            let tools = Tools::default();
            let options = super::ops::PutOptions {
                host: &host,
                local_path: &local,
                remote: &remote,
                timeout: fs_timeout(timeout_nanos),
                resume,
                checksum,
                parents,
            };
            let result = super::ops::put(client, &tools, &options, stop);
            match &result {
                Ok(_) => timer.succeeded("", &format!("put {local} -> {remote}"), None),
                Err(error) => timer.failed(error.code),
            }
            deliver(
                sink,
                json,
                "fs.put",
                &host,
                result,
                |sink, result| sink.envelope(&dto::transfer("fs.put", &host, result)),
                render::transfer,
            )
        }
        Fs::Get {
            host,
            remote,
            local,
            timeout_nanos,
            resume,
            checksum,
        } => {
            let timer = Timer::start("fs.get", &host);
            let tools = Tools::default();
            let options = super::ops::GetOptions {
                host: &host,
                remote: &remote,
                local_path: &local,
                timeout: fs_timeout(timeout_nanos),
                resume,
                checksum,
            };
            let result = super::ops::get(client, &tools, &options, stop);
            match &result {
                Ok(_) => timer.succeeded("", &format!("get {remote} -> {local}"), None),
                Err(error) => timer.failed(error.code),
            }
            deliver(
                sink,
                json,
                "fs.get",
                &host,
                result,
                |sink, result| sink.envelope(&dto::transfer("fs.get", &host, result)),
                render::transfer,
            )
        }
        Fs::Sync {
            mirror,
            host,
            source,
            destination,
            timeout_nanos,
            delete,
            dry_run,
            checksum,
            excludes,
        } => {
            let tools = Tools::default();
            let options = super::ops::SyncOptions {
                host: &host,
                source: &source,
                destination: &destination,
                delete,
                dry_run,
                excludes: &excludes,
                checksum,
                timeout: fs_timeout(timeout_nanos),
            };
            if mirror {
                let timer = Timer::start("fs.mirror", &host);
                let result = super::ops::mirror(client, &tools, &options, stop);
                match &result {
                    Ok(_) => timer.succeeded("", &format!("mirror {source}"), None),
                    Err(error) => timer.failed(error.code),
                }
                deliver(
                    sink,
                    json,
                    "fs.mirror",
                    &host,
                    result,
                    |sink, result| sink.envelope(&dto::sync("fs.mirror", &host, result)),
                    |sink, result| render::sync(sink, result, "downloaded", "would download"),
                )
            } else {
                let timer = Timer::start("fs.sync", &host);
                let result = super::ops::sync(client, &tools, &options, stop);
                match &result {
                    Ok(_) => {
                        let mut summary = format!("sync {source} -> {destination}");
                        if delete {
                            summary.push_str(" --delete");
                        }
                        if dry_run {
                            summary.push_str(" --dry-run");
                        }
                        timer.succeeded("", &summary, None);
                    }
                    Err(error) => timer.failed(error.code),
                }
                deliver(
                    sink,
                    json,
                    "fs.sync",
                    &host,
                    result,
                    |sink, result| sink.envelope(&dto::sync("fs.sync", &host, result)),
                    |sink, result| render::sync(sink, result, "synced", "would sync"),
                )
            }
        }
        Fs::Batch {
            host,
            manifest,
            timeout_nanos,
        } => {
            let tools = Tools::default();
            match super::ops::batch(
                client,
                &tools,
                &host,
                &manifest,
                fs_timeout(timeout_nanos),
                stop,
            ) {
                Ok(report) => {
                    // One entry per copy, like the reference: the batch is
                    // orchestration, and what a reader wants to know is which
                    // individual copies this machine attempted.
                    for item in &report.items {
                        let operation = if item.operation == "get" {
                            "fs.get"
                        } else {
                            "fs.put"
                        };
                        let endpoint = if item.operation == "get" {
                            &item.source
                        } else {
                            &item.destination
                        };
                        let entry = match (&item.data, &item.error) {
                            (Some(_), _) => audit::Entry::completed(
                                operation,
                                &host,
                                "",
                                &format!("{} {endpoint}", item.operation),
                                None,
                                item.duration_ms,
                            ),
                            (_, Some(error)) => audit::Entry::refused(
                                operation,
                                &host,
                                error.code,
                                item.duration_ms,
                            ),
                            // Neither is impossible by construction, but a
                            // result that says nothing is not worth recording.
                            _ => continue,
                        };
                        record(entry);
                    }
                    if json {
                        sink.envelope(&dto::batch(&host, &report));
                    } else {
                        render::batch(sink, &report);
                    }
                    // The batch ran to the end; a failed entry still makes the
                    // process status non-zero, because "the batch completed"
                    // and "every file arrived" are different claims.
                    if report.failed > 0 { 255 } else { 0 }
                }
                Err(error) => Failure::from_error("fs.batch", &host, error).deliver(sink, json),
            }
        }
        Fs::Read {
            host,
            path,
            start,
            lines,
            max_bytes,
            timeout_nanos,
        } => {
            let options = super::ops::ReadOptions {
                host: &host,
                path: &path,
                start,
                lines,
                max_bytes,
                timeout: fs_timeout(timeout_nanos),
            };
            deliver(
                sink,
                json,
                "fs.read",
                &host,
                super::ops::read(client, &options),
                |sink, result| sink.envelope(&dto::file_read(&host, result)),
                render::read,
            )
        }
        Fs::Write {
            host,
            path,
            from,
            if_hash,
            parents,
            mode,
            timeout_nanos,
        } => {
            // The body is read locally before anything is sent, so a wrong
            // path is a configuration answer and not a half-run operation.
            match read_local_input(&from) {
                Err(error) => Failure::from_error("fs.write", &host, error).deliver(sink, json),
                Ok(content) => {
                    let timer = Timer::start("fs.write", &host);
                    let options = super::ops::WriteOptions {
                        host: &host,
                        path: &path,
                        content,
                        if_hash,
                        parents,
                        mode,
                        timeout: fs_timeout(timeout_nanos),
                    };
                    let result = super::ops::write(client, &options);
                    match &result {
                        Ok(_) => timer.succeeded("", &format!("write {path}"), None),
                        Err(error) => timer.failed(error.code),
                    }
                    deliver(
                        sink,
                        json,
                        "fs.write",
                        &host,
                        result,
                        |sink, result| sink.envelope(&dto::file_write("fs.write", &host, result)),
                        render::write,
                    )
                }
            }
        }
        Fs::Patch {
            host,
            path,
            patch,
            if_hash,
            timeout_nanos,
        } => match read_local_input(&patch)
            .and_then(|document| parse_patch_document(&document, if_hash.as_deref()))
        {
            Err(error) => Failure::from_error("fs.patch", &host, error).deliver(sink, json),
            Ok((edits, carried)) => {
                let timer = Timer::start("fs.patch", &host);
                let result = super::ops::patch(
                    client,
                    &host,
                    &path,
                    edits,
                    carried,
                    256 * 1024,
                    fs_timeout(timeout_nanos),
                );
                match &result {
                    Ok(_) => timer.succeeded("", &format!("patch {path}"), None),
                    Err(error) => timer.failed(error.code),
                }
                deliver(
                    sink,
                    json,
                    "fs.patch",
                    &host,
                    result,
                    |sink, result| sink.envelope(&dto::file_write("fs.patch", &host, result)),
                    render::write,
                )
            }
        },
    }
}

/// `0` means "the operation's own default": a transfer's default is minutes,
/// while an editing helper's is one bounded action.
fn fs_timeout(nanos: i64) -> Option<Duration> {
    (nanos > 0).then(|| Duration::from_nanos(nanos as u64))
}

/// Delivers one file-operation result: the envelope an agent asked for, or the
/// human view, or the failure both agree on.
fn deliver<T>(
    sink: &mut Sink,
    json: bool,
    operation: &'static str,
    host: &str,
    result: Result<T, Error>,
    envelope: impl FnOnce(&mut Sink, &T),
    human: impl FnOnce(&mut Sink, &T),
) -> u8 {
    match result {
        Ok(value) => {
            if json {
                envelope(sink, &value);
            } else {
                human(sink, &value);
            }
            0
        }
        Err(error) => Failure::from_error(operation, host, error).deliver(sink, json),
    }
}
