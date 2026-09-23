//! Best-effort discovery of configured SSH targets.
//!
//! OpenSSH configuration remains the source of truth (`ssh <target>` is the
//! authority); this exists so an agent can see what the user probably meant.
//! It follows `Include` directives and skips wildcard or negated patterns,
//! because they do not name a single host. It does not implement `Match`,
//! quoted includes, or the system-wide file, and it never contacts a host.
//! Anything it could not read is reported through `complete` and `warnings`
//! rather than presented as an empty list.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub alias: String,
    pub source: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub hosts: Vec<Info>,
    pub config_found: bool,
    pub complete: bool,
    pub warnings: Vec<String>,
}

pub fn aliases() -> Discovery {
    match crate::config::home_dir() {
        Some(home) => aliases_from(&home),
        None => Discovery {
            hosts: Vec::new(),
            config_found: false,
            // No HOME means nothing was read, which is not the same as having
            // read everything.
            complete: false,
            warnings: vec!["HOME is unset, so the OpenSSH client config was not read".into()],
        },
    }
}

pub fn aliases_from(home: &Path) -> Discovery {
    let base = home.join(".ssh").join("config");
    let mut discovery = Discovery {
        hosts: Vec::new(),
        config_found: false,
        complete: true,
        warnings: Vec::new(),
    };
    match std::fs::symlink_metadata(&base) {
        Ok(_) => discovery.config_found = true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            discovery.complete = false;
            discovery
                .warnings
                .push("could not inspect the OpenSSH client config file".into());
            return discovery;
        }
    }
    // `ssh` is the authority for everything else: a missing user config is an
    // empty answer, not an error, and no host is contacted either way.
    if discovery.config_found {
        let mut seen: Vec<String> = Vec::new();
        let mut visited: Vec<PathBuf> = Vec::new();
        walk(&base, home, &mut seen, &mut visited, &mut discovery);
    }
    discovery.hosts.sort_by(|a, b| a.alias.cmp(&b.alias));
    discovery
}

fn walk(
    path: &Path,
    home: &Path,
    seen: &mut Vec<String>,
    visited: &mut Vec<PathBuf>,
    discovery: &mut Discovery,
) {
    let absolute = match path.canonicalize() {
        Ok(absolute) => absolute,
        Err(_) => PathBuf::from(path),
    };
    if visited.contains(&absolute) {
        return;
    }
    visited.push(absolute);
    let Ok(contents) = std::fs::read_to_string(path) else {
        discovery.complete = false;
        discovery
            .warnings
            .push("could not read an included SSH config file".into());
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (keyword, rest) = split_keyword(line);
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                for token in rest.split_whitespace() {
                    if token.is_empty() || token.contains(['*', '?', '!']) {
                        continue;
                    }
                    if seen.iter().any(|known| known == token) {
                        continue;
                    }
                    seen.push(token.to_string());
                    discovery.hosts.push(Info {
                        alias: token.to_string(),
                        source: Some(path.to_string_lossy().into_owned()),
                    });
                }
            }
            "include" => {
                for pattern in rest.split_whitespace() {
                    match expand(pattern, home) {
                        Ok(matches) => {
                            for found in matches {
                                walk(&found, home, seen, visited, discovery);
                            }
                        }
                        Err(reason) => {
                            discovery.complete = false;
                            discovery.warnings.push(reason);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Splits an ssh_config line into keyword and remainder, accepting both
/// `Key value` and `Key=value`.
fn split_keyword(line: &str) -> (String, String) {
    let line = line.trim();
    match line.find([' ', '\t', '=']) {
        None => (line.to_string(), String::new()),
        Some(index) => (
            line[..index].to_string(),
            line[index..]
                .trim_matches(|c| c == ' ' || c == '\t' || c == '=')
                .to_string(),
        ),
    }
}

/// Expands an `Include` pattern: absolute paths are used as given, everything
/// else is relative to `~/.ssh`, matching OpenSSH's own semantics.
fn expand(pattern: &str, home: &Path) -> Result<Vec<PathBuf>, String> {
    let rooted = if let Some(rest) = pattern.strip_prefix("~/") {
        home.join(rest)
    } else if Path::new(pattern).is_absolute() {
        PathBuf::from(pattern)
    } else {
        home.join(".ssh").join(pattern)
    };
    let text = rooted.to_string_lossy();
    if text.contains(['[', '{']) {
        return Err(format!(
            "unsupported Include pattern (only * and ? are matched): {pattern}"
        ));
    }
    Ok(glob(&rooted))
}

/// Matches `*` and `?` within one path component at a time.
fn glob(pattern: &Path) -> Vec<PathBuf> {
    let mut current: Vec<PathBuf> = vec![PathBuf::from("/")];
    let root = if pattern.is_absolute() {
        PathBuf::from("/")
    } else {
        PathBuf::from(".")
    };
    current[0] = root;
    for component in pattern.components() {
        let std::path::Component::Normal(name) = component else {
            if matches!(
                component,
                std::path::Component::RootDir | std::path::Component::CurDir
            ) {
                continue;
            }
            // A pattern that walks up or out cannot be expanded safely here.
            return Vec::new();
        };
        let text = name.to_string_lossy();
        if !text.contains(['*', '?']) {
            current = current.into_iter().map(|base| base.join(name)).collect();
            continue;
        }
        let mut next: Vec<PathBuf> = Vec::new();
        for base in current {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            for entry in entries.flatten() {
                let candidate = entry.file_name().to_string_lossy().into_owned();
                if matches_component(&candidate, &text) {
                    next.push(base.join(entry.file_name()));
                }
            }
        }
        current = next;
    }
    current
        .into_iter()
        .filter(|candidate| candidate.is_file())
        .collect()
}

fn matches_component(value: &str, pattern: &str) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    fn walk_matching(value: &[char], pattern: &[char]) -> bool {
        if pattern.is_empty() {
            return value.is_empty();
        }
        match pattern[0] {
            '*' => (0..=value.len()).any(|skip| walk_matching(&value[skip..], &pattern[1..])),
            '?' if !value.is_empty() => walk_matching(&value[1..], &pattern[1..]),
            other if !value.is_empty() && value[0] == other => {
                walk_matching(&value[1..], &pattern[1..])
            }
            _ => false,
        }
    }
    walk_matching(&value, &pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rhost-hosts-{}", std::process::id()));
        let dir = root.join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".ssh")).unwrap_or_else(|error| panic!("{error}"));
        dir
    }

    #[test]
    fn aliases_follow_includes_and_skip_patterns() {
        let home = fixture("includes");
        let ssh = home.join(".ssh");
        fs::write(
            ssh.join("config"),
            "# comment\nHost alpha\n  HostName a.example\nhost  beta gamma*\nHost=delta\nInclude config.d/*\nInclude broken/[abc]\n",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        fs::create_dir_all(ssh.join("config.d")).unwrap_or_else(|error| panic!("{error}"));
        fs::write(
            ssh.join("config.d").join("more"),
            "Host epsilon\n  Include ../config\n",
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let found = aliases_from(&home);
        let aliases: Vec<&str> = found.hosts.iter().map(|info| info.alias.as_str()).collect();
        // `gamma*` names no single host, so it is skipped; `beta` is a real
        // alias even though its sibling pattern is not.
        assert_eq!(aliases, ["alpha", "beta", "delta", "epsilon"]);
        assert!(found.config_found);
        assert!(
            !found.complete,
            "an Include this reader cannot expand must lower completeness"
        );
        assert!(!found.warnings.is_empty());
        assert_eq!(
            found.hosts.first().map(|info| info.source.as_deref()),
            Some(Some(ssh.join("config").to_string_lossy().as_ref()))
        );
    }

    #[test]
    fn an_absent_config_is_an_empty_but_honest_answer() {
        let home = fixture("absent");
        let found = aliases_from(&home);
        assert!(found.hosts.is_empty());
        assert!(!found.config_found);
        assert!(found.complete);
        assert!(found.warnings.is_empty());
    }

    #[test]
    fn keywords_and_globs_are_matched_like_openssh_expects() {
        assert_eq!(split_keyword("Host=a b"), ("Host".into(), "a b".into()));
        assert_eq!(split_keyword("  host   x  "), ("host".into(), "x".into()));
        assert!(matches_component("config.d", "config*"));
        assert!(!matches_component("other", "config*"));
        assert!(matches_component("a", "?"));
        assert!(!matches_component("ab", "?"));
    }
}
