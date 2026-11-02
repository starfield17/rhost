//! Bounded observation over remote-owned Run records, shared by all adapters.
use crate::{
    helper::{Helper, Writer},
    remote::Error,
    transport::Client,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Watch {
    generation: String,
    run_id: String,
    #[serde(default = "wait")]
    wait_ms: u32,
}
fn wait() -> u32 {
    30000
}
fn optional<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}
fn cap() -> u32 {
    4096
}
fn matches() -> usize {
    32
}
fn context() -> usize {
    2
}
fn format() -> String {
    "text".into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    generation: String,
    run_id: String,
    stream: String,
    #[serde(default, deserialize_with = "optional")]
    since: Option<u64>,
    #[serde(default, deserialize_with = "optional")]
    tail: Option<u64>,
    #[serde(default, deserialize_with = "optional")]
    until: Option<u64>,
    #[serde(default = "cap")]
    max_bytes: u32,
    #[serde(default = "format")]
    format: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    generation: String,
    run_id: String,
    stream: String,
    pattern: String,
    #[serde(default)]
    since: u64,
    #[serde(default, deserialize_with = "optional")]
    until: Option<u64>,
    #[serde(default = "cap")]
    max_bytes: u32,
    #[serde(default = "matches")]
    max_matches: usize,
    #[serde(default = "context")]
    context_lines: usize,
}
fn invalid(s: impl Into<String>) -> Error {
    Error::new("CONFIG_INVALID", s).with_recovery(crate::domain::Recovery::invalid())
}
fn parse<T: for<'a> Deserialize<'a>>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|e| invalid(e.to_string()))
}
fn identity(g: &str, k: &str) -> Result<Writer, Error> {
    super::request::generation(g)?;
    super::request::key(k)?;
    let mut b = Writer::default();
    b.text(g)?;
    b.text(k)?;
    Ok(b)
}
pub(super) fn execute(
    client: &Client,
    host: &str,
    op: &str,
    raw: &str,
    cancelled: &dyn Fn() -> bool,
    guard: Option<&crate::domain::BootId>,
) -> Result<Value, Error> {
    let helper = Helper::new(client, host)
        .with_cancellation(cancelled)
        .with_boot_guard(guard);
    if op == "run.watch" {
        let p: Watch = parse(raw)?;
        if p.wait_ms > 300000 {
            return Err(invalid("invalid wait budget"));
        }
        let mut b = identity(&p.generation, &p.run_id)?;
        b.int(p.wait_ms);
        let (_, reply) = helper.operation(9, b, p.wait_ms)?;
        let data = super::dto::decode(9, &reply)?;
        super::dto::bind(9, &data, Some(&p.generation), Some(&p.run_id))?;
        return Ok(data);
    }
    let (p, search) = if op == "run.search" {
        let s: Search = parse(raw)?;
        if s.pattern.len() > s.max_bytes as usize
            || !(4..=1048576).contains(&s.max_bytes)
            || s.pattern.is_empty()
            || s.pattern.len() > 4096
            || s.pattern.contains(['\n', '\r', '\0'])
            || !(1..=128).contains(&s.max_matches)
            || s.context_lines > 20
        {
            return Err(invalid("invalid search bounds"));
        }
        (
            Read {
                generation: s.generation.clone(),
                run_id: s.run_id.clone(),
                stream: s.stream.clone(),
                since: Some(s.since),
                tail: None,
                until: s.until,
                max_bytes: s.max_bytes.max(s.pattern.len() as u32 + 4),
                format: format(),
            },
            Some(s),
        )
    } else {
        (parse::<Read>(raw)?, None)
    };
    if !(4..=1048576).contains(&p.max_bytes)
        || p.since.is_some() && p.tail.is_some()
        || !matches!(p.format.as_str(), "text" | "base64")
    {
        return Err(invalid("invalid output selection"));
    }
    let mut b = identity(&p.generation, &p.run_id)?;
    b.byte(match p.stream.as_str() {
        "stdout" => 1,
        "stderr" => 2,
        _ => return Err(invalid("stream must be stdout or stderr")),
    });
    b.byte(u8::from(p.since.is_none()) | (u8::from(p.until.is_none()) << 1));
    b.long(p.since.unwrap_or(0));
    b.long(p.tail.unwrap_or(u64::from(p.max_bytes)));
    b.long(p.until.unwrap_or(0));
    b.int(p.max_bytes);
    let (_, reply) = helper.operation(10, b, 0)?;
    let mut data = super::dto::decode(10, &reply)?;
    super::dto::bind(10, &data, Some(&p.generation), Some(&p.run_id))?;
    let out = &mut data["output"];
    if out["stream"] != p.stream {
        return Err(Error::new("PROTOCOL_ERROR", "foreign stream response"));
    }
    let encoded = out["content_base64"]
        .as_str()
        .ok_or_else(|| invalid("missing stream bytes"))?;
    let bytes = crate::base64::decode(encoded).ok_or_else(|| invalid("invalid stream bytes"))?;
    let kept = out["retained_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("missing retained count"))?;
    let expected_end = p.until.unwrap_or(kept);
    let expected_from = p
        .since
        .unwrap_or_else(|| expected_end.saturating_sub(p.tail.unwrap_or(u64::from(p.max_bytes))));
    if out["from"] != expected_from
        || out["until"] != expected_end
        || bytes.len() > p.max_bytes as usize
    {
        return Err(Error::new("PROTOCOL_ERROR", "foreign output range"));
    }
    let from = out["from"]
        .as_u64()
        .ok_or_else(|| invalid("missing cursor"))?;
    let until = out["until"]
        .as_u64()
        .ok_or_else(|| invalid("missing endpoint"))?;
    if let Some(s) = search {
        return search_page(data, &bytes, from, until, &s);
    }
    let mut start = 0;
    let mut end = bytes.len();
    if p.format == "text" {
        if p.since.is_none() {
            while start < end && bytes[start] & 0xc0 == 0x80 {
                start += 1;
            }
        }
        end -= crate::shell::incomplete_utf8_suffix(&bytes[start..end]);
        if start == end && !bytes.is_empty() {
            return Err(Error::new(
                "INVALID_TEXT",
                "incomplete UTF-8 page; use base64 or observe a newer endpoint",
            ));
        }
        let text = std::str::from_utf8(&bytes[start..end])
            .map_err(|_| Error::new("INVALID_TEXT", "use format base64 for non-UTF-8 output"))?;
        out["content"] = json!(text);
    } else {
        out["content"] = json!(crate::base64::encode(&bytes));
    }
    let next = from + end as u64;
    // A fixed endpoint can split a character, including at the retention limit.
    // Preserve the remaining raw bytes through an executable binary request.
    let next_format = if end < bytes.len() && from + bytes.len() as u64 == until {
        "base64"
    } else {
        p.format.as_str()
    };
    let delivered = from + start as u64;
    out["from"] = json!(delivered);
    out["next"] = json!(next);
    out["format"] = json!(p.format);
    let observed = out["observed_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("missing observed size"))?;
    let kept = out["retained_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("missing retained size"))?;
    out["coverage"] = json!({"from":delivered,"next":next,"observed_bytes":observed,"more":next<until,"omitted_before":delivered,"unavailable_after":observed-kept,"next_request":if next<until {Some(json!({"operation":"run.read","params":{"generation":p.generation,"run_id":p.run_id,"stream":p.stream,"since":next,"until":until,"max_bytes":p.max_bytes,"format":next_format}}))}else{None}});
    out.as_object_mut()
        .ok_or_else(|| invalid("stream object"))?
        .remove("content_base64");
    Ok(data)
}
fn search_page(
    data: Value,
    bytes: &[u8],
    from: u64,
    until: u64,
    s: &Search,
) -> Result<Value, Error> {
    let pattern = s.pattern.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    let mut budget = s.max_bytes as usize;
    while at + pattern.len() <= bytes.len() {
        if &bytes[at..at + pattern.len()] == pattern {
            if found.len() == s.max_matches {
                break;
            }
            let mut lo = at;
            let mut hi = at + pattern.len();
            for _ in 0..=s.context_lines {
                while lo > 0 && bytes[lo - 1] != b'\n' {
                    lo -= 1;
                }
                lo = lo.saturating_sub(1);
                while hi < bytes.len() && bytes[hi] != b'\n' {
                    hi += 1;
                }
                if hi < bytes.len() {
                    hi += 1;
                }
            }
            if hi - lo > budget {
                lo = at;
                hi = (at + pattern.len()).min(bytes.len());
            }
            if hi - lo > budget {
                break;
            }
            while lo < hi && bytes[lo] & 0xc0 == 0x80 {
                lo += 1;
            }
            if from + (hi as u64) <= until {
                hi -= crate::shell::incomplete_utf8_suffix(&bytes[lo..hi]);
            }
            let text = std::str::from_utf8(&bytes[lo..hi])
                .map_err(|_| Error::new("INVALID_TEXT", "log search requires UTF-8 text"))?;
            found.push(json!({"from":from+lo as u64,"next":from+hi as u64,"text":text}));
            budget -= hi - lo;
            at += 1;
        } else {
            at += 1;
        }
    }
    let scanned = from + bytes.len() as u64;
    let next = if at + pattern.len() > bytes.len() {
        if scanned < until {
            scanned
                .saturating_sub(pattern.len().saturating_sub(1) as u64)
                .max(from + 1)
        } else {
            scanned
        }
    } else {
        from + at as u64
    };
    Ok(
        json!({"run":data["run"],"stream":s.stream,"from":from,"next":next,"until":until,"matches":found,"more":next<until,"incomplete":data["output"]["incomplete"]}),
    )
}
