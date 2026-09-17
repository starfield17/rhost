//! Whole trees: `sync` pushes a local directory, `mirror` pulls a remote one.
//!
//! Nothing is deleted unless `--delete` was asked for, and a destructive sync is
//! refused twice: once from the text the caller typed and once from what the
//! remote path actually resolves to (FS-005).

use super::super::backend;
use super::super::backend::{Stop, Tools};
use super::remote::{helper, master_alive, remote_has_rsync, validation};
use super::transfer::run_tool;
use super::{HELPER_TIMEOUT, Sync, SyncOptions};
use crate::remote::Error;
use crate::remote::exec;
use crate::transport::Client;
use std::path::Path;
use std::time::Duration;

fn stat_local_dir(path: &Path) -> Result<(), Error> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        Error::new(
            "CONFIG_INVALID",
            format!("cannot read {}: {error}", path.display()),
        )
    })?;
    if !metadata.is_dir() {
        return Err(Error::new(
            "CONFIG_INVALID",
            "fs sync takes a local directory; use fs put for one file",
        ));
    }
    Ok(())
}

fn summarize(
    source: &str,
    destination: &str,
    multiplexed: bool,
    options: &SyncOptions<'_>,
    outcome: &backend::Outcome,
) -> Sync {
    let (changes, notes) = backend::parse_changes(&outcome.stdout_text());
    let mut sync = Sync {
        source: source.to_string(),
        destination: destination.to_string(),
        dry_run: options.dry_run,
        delete: options.delete,
        multiplexed,
        files: 0,
        directories: 0,
        deletes: 0,
        changes,
        notes,
        duration_ms: outcome.duration.as_millis() as u64,
    };
    for change in &sync.changes {
        match change.action {
            backend::Action::Delete => sync.deletes += 1,
            backend::Action::Create | backend::Action::Update => sync.files += 1,
            backend::Action::Directory => sync.directories += 1,
            backend::Action::Skip | backend::Action::Other => {}
        }
    }
    sync
}

/// The real destination of a destructive sync, as the remote host resolves it.
pub(crate) fn resolve(
    client: &Client,
    host: &str,
    path: &str,
    delete: bool,
    timeout: Duration,
) -> Result<String, Error> {
    let answer: backend::ResolveResult = helper(
        client,
        host,
        &backend::Request::Resolve { path, delete },
        timeout.min(HELPER_TIMEOUT),
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    Ok(answer.path)
}

/// Brings a remote directory in line with a local one.
pub fn sync(
    client: &Client,
    tools: &Tools,
    options: &SyncOptions<'_>,
    stop: Stop<'_>,
) -> Result<Sync, Error> {
    backend::validate_remote_path(options.destination).map_err(validation)?;
    let local = backend::local_arg(options.source).map_err(validation)?;
    let mut request = backend::SyncRequest {
        host: options.host.to_string(),
        source: local.clone(),
        destination: options.destination.to_string(),
        delete: options.delete,
        dry_run: options.dry_run,
        excludes: options.excludes.to_vec(),
    };
    let mut plan = backend::push_args(&request, &client.ssh_options()).map_err(validation)?;
    stat_local_dir(Path::new(&local))?;
    remote_has_rsync(client, options.host)?;
    if options.delete {
        // A local check cannot see that the remote destination is a symlink to
        // `/`, so a destructive sync asks the remote what the path actually
        // names and prunes *that*.
        let actual = resolve(
            client,
            options.host,
            options.destination,
            true,
            options.timeout.unwrap_or(backend::DEFAULT_TIMEOUT),
        )?;
        request.destination = actual;
        plan = backend::push_args(&request, &client.ssh_options()).map_err(validation)?;
    }
    if options.checksum {
        plan.args.insert(0, "--checksum".to_string());
    }
    let outcome = run_tool(tools, &tools.rsync, &plan.args, options.timeout, stop)?;
    Ok(summarize(
        &plan.source,
        &plan.destination,
        plan.multiplexed && master_alive(client, options.host),
        options,
        &outcome,
    ))
}

/// Downloads the contents of a remote directory into a local one.
pub fn mirror(
    client: &Client,
    tools: &Tools,
    options: &SyncOptions<'_>,
    stop: Stop<'_>,
) -> Result<Sync, Error> {
    let absolute = std::path::absolute(options.destination)
        .map_err(|error| Error::new("CONFIG_INVALID", error.to_string()))?;
    let mut local = absolute.to_string_lossy().to_string();
    backend::reject_sync_target(&local, options.delete).map_err(validation)?;
    if options.delete {
        // The guard is applied to what the path *is*, after symlinks, for the
        // same reason as on the remote side. A destination that does not exist
        // yet has nothing to prune, so there is nothing to resolve.
        match std::fs::symlink_metadata(&local) {
            Ok(_) => {
                let actual = std::fs::canonicalize(&local)
                    .map_err(|error| Error::new("SYNC_REJECTED", error.to_string()))?;
                let actual = actual.to_string_lossy().to_string();
                if crate::config::home_dir()
                    .is_some_and(|home| home.to_string_lossy().as_ref() == actual.as_str())
                {
                    return Err(Error::new(
                        "SYNC_REJECTED",
                        "--delete refuses an entire home directory; mirror into a subdirectory",
                    ));
                }
                backend::reject_sync_target(&actual, true).map_err(validation)?;
                local = actual;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::new("SYNC_REJECTED", error.to_string())),
        }
    }
    if options.source.trim().is_empty() {
        return Err(Error::new(
            "CONFIG_INVALID",
            "a mirror needs a remote source directory",
        ));
    }
    remote_has_rsync(client, options.host)?;
    let request = backend::SyncRequest {
        host: options.host.to_string(),
        source: options.source.to_string(),
        destination: local.clone(),
        delete: options.delete,
        dry_run: options.dry_run,
        excludes: options.excludes.to_vec(),
    };
    let mut plan = backend::pull_args(&request, &client.ssh_options()).map_err(validation)?;
    if options.checksum {
        plan.args.insert(0, "--checksum".to_string());
    }
    let outcome = run_tool(tools, &tools.rsync, &plan.args, options.timeout, stop)?;
    Ok(summarize(
        &plan.source,
        &plan.destination,
        plan.multiplexed && master_alive(client, options.host),
        options,
        &outcome,
    ))
}
