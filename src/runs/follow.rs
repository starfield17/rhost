//! Correlate retained bytes and independent leader evidence on RH02.
use crate::helper::{Helper, Reader, Writer, codec::protocol, mux};
use crate::remote::Error;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Watch {
    pub generation: String,
    pub run_id: String,
    #[serde(default = "wait")]
    wait_ms: u32,
    #[serde(default, deserialize_with = "specified")]
    stdout_since: Option<u64>,
    #[serde(default, deserialize_with = "specified")]
    stderr_since: Option<u64>,
    #[serde(default, deserialize_with = "specified")]
    max_bytes: Option<u32>,
}

fn specified<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

fn wait() -> u32 {
    30000
}

struct Page {
    from: u64,
    bytes: Vec<u8>,
}

impl Page {
    fn next(&self) -> Result<u64, Error> {
        self.from
            .checked_add(self.bytes.len() as u64)
            .ok_or_else(|| protocol("FOLLOW offset overflow"))
    }

    fn append(&mut self, from: u64, bytes: &[u8], cap: usize) -> Result<(), Error> {
        if self.next()? != from || bytes.is_empty() || self.bytes.len() + bytes.len() > cap {
            return Err(protocol("inconsistent FOLLOW data frame"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn finish(&self, output: &mut Value, enabled: bool) -> Result<(), Error> {
        let retained = output["retained_bytes"]
            .as_u64()
            .ok_or_else(|| protocol("missing retained offset"))?;
        let observed = output["observed_bytes"]
            .as_u64()
            .ok_or_else(|| protocol("missing observed offset"))?;
        output["unavailable_after"] = json!(observed - retained);
        if enabled {
            let next = self.next()?;
            if next > retained {
                return Err(protocol("FOLLOW data exceeds published retention"));
            }
            output["from"] = json!(self.from);
            output["next"] = json!(next);
            output["content_base64"] = json!(crate::base64::encode(&self.bytes));
        }
        Ok(())
    }
}

impl Watch {
    pub fn execute(
        &self,
        helper: &Helper<'_>,
        pool: Option<&mux::Pool>,
        mut body: Writer,
    ) -> Result<Value, Error> {
        let enabled =
            self.stdout_since.is_some() || self.stderr_since.is_some() || self.max_bytes.is_some();
        let cap = if enabled {
            self.max_bytes.unwrap_or(4096)
        } else {
            0
        };
        if self.wait_ms > 300000 || cap > 1048576 || self.max_bytes == Some(0) {
            return Err(Error::new("CONFIG_INVALID", "invalid FOLLOW bounds")
                .with_recovery(crate::domain::Recovery::invalid()));
        }
        let mut pages = [
            self.stdout_since.unwrap_or(0),
            self.stderr_since.unwrap_or(0),
        ]
        .map(|from| Page {
            from,
            bytes: Vec::new(),
        });
        body.long(pages[0].from);
        body.long(pages[1].from);
        body.int(self.wait_ms);
        body.int(cap);
        let mut completion = None;
        let reply = helper.mux(
            pool,
            mux::FOLLOW,
            body,
            self.wait_ms,
            |kind, bytes| match kind {
                mux::DATA if enabled => {
                    let mut r = Reader::new(bytes);
                    let page = match r.byte()? {
                        1 => &mut pages[0],
                        2 => &mut pages[1],
                        _ => return Err(protocol("invalid FOLLOW stream")),
                    };
                    let from = r.long()?;
                    let bytes = r.bytes()?;
                    r.done()?;
                    page.append(from, bytes, cap as usize)
                }
                mux::COMPLETION if enabled && completion.is_none() => {
                    let evidence = self.decode(kind, &crate::helper::codec::result(bytes)?)?;
                    if evidence["run"]["status"] != "exited" {
                        return Err(protocol("invalid FOLLOW leader completion"));
                    }
                    completion = Some(evidence["run"]["completion"].clone());
                    Ok(())
                }
                _ => Err(protocol("unexpected FOLLOW event")),
            },
        )?;
        let mut data = self.decode(mux::FOLLOW, &reply)?;
        if completion.is_some_and(|value| data["run"]["completion"] != value) {
            return Err(protocol("FOLLOW leader completion changed"));
        }
        for (stream, page) in ["stdout", "stderr"].into_iter().zip(pages) {
            page.finish(&mut data[stream], enabled)?;
        }
        Ok(data)
    }

    fn decode(&self, kind: u32, body: &[u8]) -> Result<Value, Error> {
        let value = super::dto::decode(kind, body, true)?;
        super::dto::bind(kind, &value, Some(&self.generation), Some(&self.run_id))?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_pages_reject_gaps_and_preserve_raw_resume_offsets() -> Result<(), Error> {
        let mut page = Page {
            from: 12,
            bytes: Vec::new(),
        };
        assert!(page.append(13, b"skip", 5).is_err());
        page.append(12, &[0xff, 0], 5)?;
        page.append(14, b"abc", 5)?;
        assert!(page.append(17, b"x", 5).is_err());
        let mut output = json!({"retained_bytes":20,"observed_bytes":22});
        page.finish(&mut output, true)?;
        assert_eq!(output["next"], 17);
        assert_eq!(output["unavailable_after"], 2);
        assert_eq!(
            crate::base64::decode(output["content_base64"].as_str().unwrap_or_default()),
            Some(vec![0xff, 0, b'a', b'b', b'c'])
        );
        Ok(())
    }
}
