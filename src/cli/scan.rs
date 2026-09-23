//! Argument scanning without whole-invocation routing.

use super::flags::FlagSpec;

pub struct Flag {
    name: &'static str,
    value: String,
}

pub struct Parsed {
    pub operands: Vec<String>,
    pub flags: Vec<Flag>,
    /// A `--` separator was seen. Its presence matters: `exec` refuses operands
    /// after it rather than guessing which one was meant as a command.
    pub dash: bool,
}

impl Parsed {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|flag| flag.name == name)
            .map(|flag| flag.value.as_str())
    }

    pub fn occurrences(&self, name: &str) -> usize {
        self.flags.iter().filter(|flag| flag.name == name).count()
    }

    pub fn all(&self, name: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|flag| flag.name == name)
            .map(|flag| flag.value.clone())
            .collect()
    }

    pub fn bool_flag(&self, name: &str) -> bool {
        self.value(name) == Some("true")
    }

    pub fn operand(&self, index: usize) -> Option<&str> {
        self.operands.get(index).map(String::as_str)
    }
}

/// Splits argv into flags and operands.
///
/// A value flag takes the next argument whether or not it looks like a flag,
/// which is what the original flag parser does; `--cwd --json` therefore sets
/// cwd to the literal `--json` instead of being guessed at. Errors come back in
/// left-to-right order, so the first thing a caller mistyped is the first thing
/// reported.
pub fn parse(argv: &[String], specs: &[FlagSpec]) -> Result<Parsed, String> {
    let mut operands: Vec<String> = Vec::new();
    let mut flags: Vec<Flag> = Vec::new();
    let mut dash = false;
    let mut index = 0;
    while index < argv.len() {
        let Some(token) = argv.get(index) else { break };
        index += 1;
        if dash {
            operands.push(token.clone());
            continue;
        }
        if token == "--" {
            dash = true;
            continue;
        }
        if let Some(body) = token.strip_prefix("--") {
            let (name, explicit) = match body.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (body, None),
            };
            let Some(spec) = specs.iter().find(|spec| spec.name == name) else {
                if name == "help" {
                    flags.push(Flag {
                        name: "help",
                        value: "true".into(),
                    });
                    continue;
                }
                return Err(format!("unknown flag --{name}"));
            };
            let value = if spec.valued {
                match explicit {
                    Some(value) => value,
                    None => match argv.get(index) {
                        Some(next) => {
                            index += 1;
                            next.clone()
                        }
                        None => return Err(format!("flag --{name} needs an argument")),
                    },
                }
            } else {
                match explicit {
                    None => "true".to_string(),
                    Some(raw) if raw == "true" || raw == "false" => raw,
                    Some(raw) => {
                        return Err(format!("invalid boolean value {raw} for --{name}"));
                    }
                }
            };
            if spec.unique && flags.iter().any(|flag| flag.name == spec.name) {
                return Err(format!("--{name} may be provided exactly once"));
            }
            flags.push(Flag {
                name: spec.name,
                value,
            });
            continue;
        }
        if token.len() > 1 && token.starts_with('-') {
            let body = &token[1..];
            let mut characters = body.chars();
            let Some(character) = characters.next() else {
                continue;
            };
            let spec = match specs.iter().find(|spec| spec.short == Some(character)) {
                Some(spec) => *spec,
                None => {
                    if character == 'h' {
                        flags.push(Flag {
                            name: "help",
                            value: "true".into(),
                        });
                        continue;
                    }
                    return Err(format!("unknown flag -{character}"));
                }
            };
            let remainder: String = characters.collect();
            let value = if spec.valued {
                if remainder.is_empty() {
                    match argv.get(index) {
                        Some(next) => {
                            index += 1;
                            next.clone()
                        }
                        None => return Err(format!("flag --{} needs an argument", spec.name)),
                    }
                } else {
                    remainder
                        .strip_prefix('=')
                        .unwrap_or(&remainder)
                        .to_string()
                }
            } else if remainder.is_empty() {
                "true".to_string()
            } else {
                return Err(format!("unknown flag -{remainder}"));
            };
            if spec.unique && flags.iter().any(|flag| flag.name == spec.name) {
                return Err(format!("--{} may be provided exactly once", spec.name));
            }
            flags.push(Flag {
                name: spec.name,
                value,
            });
            continue;
        }
        operands.push(token.clone());
    }
    Ok(Parsed {
        operands,
        flags,
        dash,
    })
}

/// Locates the subcommand the way the original CLI did: a token that is neither
/// a flag nor the value of a flag is the command name, and everything after `--`
/// is an operand rather than a candidate command.
///
/// The scan uses the registered flag arities without validating scope. A
/// capability's parser still decides whether that flag belongs to its command.
pub fn find_command(argv: &[String], specs: &[FlagSpec]) -> Option<usize> {
    let mut skip_next = false;
    for (index, token) in argv.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if token == "--" {
            // Everything after the separator is an operand, never a command name.
            return None;
        }
        if let Some(body) = token.strip_prefix("--") {
            if !body.contains('=') && takes_next(token, specs) {
                skip_next = true;
            }
            continue;
        }
        let short = token.strip_prefix('-').filter(|body| !body.is_empty());
        if let Some(name) = short {
            if name.len() == 1 && takes_next(token, specs) {
                skip_next = true;
            }
            continue;
        }
        if token.is_empty() {
            continue; // an empty operand is not a command name either
        }
        return Some(index);
    }
    None
}

fn takes_next(token: &str, specs: &[FlagSpec]) -> bool {
    if let Some(name) = token.strip_prefix("--") {
        if matches!(name, "json" | "version" | "help") {
            return false;
        }
        return specs
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.valued)
            .unwrap_or(true);
    }
    if let Some(name) = token.strip_prefix('-') {
        if name == "h" {
            return false;
        }
        if name.len() == 1 {
            return specs
                .iter()
                .find(|spec| spec.short.is_some_and(|short| name.starts_with(short)))
                .map(|spec| spec.valued)
                .unwrap_or(true);
        }
    }
    false
}

/// `--json` may appear anywhere before the `--` separator, including in a
/// position where the parse itself fails.
pub fn wants_json(argv: &[String], specs: &[FlagSpec]) -> bool {
    let mut skip_next = false;
    for token in argv {
        if skip_next {
            skip_next = false;
            continue;
        }
        if token == "--" {
            break;
        }
        if token == "--json" || token == "--json=true" {
            return true;
        }
        if !token.contains('=') && takes_next(token, specs) {
            // An unknown flag does not hide a later explicit JSON request;
            // only a registered valued flag can consume it as its value.
            let name = token.trim_start_matches('-');
            if specs.iter().any(|spec| {
                spec.name == name
                    || (name.len() == 1 && spec.short.is_some_and(|short| name.starts_with(short)))
            }) {
                skip_next = true;
            }
        }
    }
    false
}

pub fn parse_duration(text: &str) -> Option<i64> {
    let negative = text.starts_with('-');
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    if body.is_empty() {
        return None;
    }
    if body == "0" {
        return Some(0);
    }
    let mut total: u128 = 0;
    let mut rest = body;
    while !rest.is_empty() {
        let length = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_digit() || *byte == b'.')
            .count();
        let digits = &rest[..length];
        if digits.is_empty() || digits == "." {
            return None;
        }
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        if fraction.contains('.') {
            return None;
        }
        let whole: u128 = if whole.is_empty() {
            0
        } else {
            whole.parse().ok()?
        };
        let fraction = fraction.trim_end_matches('0');
        rest = &rest[length..];
        let (unit, remainder) = match rest.chars().next() {
            Some('n') if rest.starts_with("ns") => ("ns", &rest[2..]),
            Some('u') if rest.starts_with("us") => ("us", &rest[2..]),
            Some('µ') if rest.starts_with("µs") => ("us", &rest[3..]),
            Some('m') if rest.starts_with("ms") => ("ms", &rest[2..]),
            Some('s') => ("s", &rest[1..]),
            Some('m') => ("m", &rest[1..]),
            Some('h') => ("h", &rest[1..]),
            _ => return None,
        };
        let scale: u128 = match unit {
            "ns" => 1,
            "us" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => return None,
        };
        let integer_nanos = whole.checked_mul(scale)?;
        let fractional_nanos = if fraction.is_empty() {
            0
        } else {
            let numerator: u128 = fraction.parse().ok()?;
            let denominator = 10u128.checked_pow(fraction.len() as u32)?;
            let scaled = numerator.checked_mul(scale)?;
            if scaled % denominator != 0 {
                return None;
            }
            scaled / denominator
        };
        total = total.checked_add(integer_nanos.checked_add(fractional_nanos)?)?;
        rest = remainder;
    }
    let limit = i64::MAX as u128 + u128::from(negative);
    if total > limit {
        return None;
    }
    if negative && total == limit {
        Some(i64::MIN)
    } else if negative {
        Some(-(total as i64))
    } else {
        Some(total as i64)
    }
}
