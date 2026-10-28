//! The bounded RH01 interface; no C memory layout crosses this boundary.
use crate::remote::Error;
pub const LIMIT: usize = 8 * 1024 * 1024;
#[derive(Default)]
pub struct Writer(pub Vec<u8>);
impl Writer {
    pub fn byte(&mut self, n: u8) {
        self.0.push(n);
    }
    pub fn int(&mut self, n: u32) {
        self.0.extend(n.to_be_bytes());
    }
    pub fn long(&mut self, n: u64) {
        self.0.extend(n.to_be_bytes());
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let size = u32::try_from(bytes.len()).map_err(|_| protocol("field exceeds limit"))?;
        self.int(size);
        self.0.extend(bytes);
        Ok(())
    }
    pub fn text(&mut self, text: &str) -> Result<(), Error> {
        if text.contains('\0') {
            return Err(protocol("NUL in text field"));
        }
        self.bytes(text.as_bytes())
    }
    pub fn frame(self, kind: u32) -> Result<Vec<u8>, Error> {
        if self.0.len() > LIMIT {
            return Err(protocol("request exceeds 8 MiB"));
        }
        let mut bytes = b"RH01".to_vec();
        bytes.extend(kind.to_be_bytes());
        bytes.extend((self.0.len() as u64).to_be_bytes());
        bytes.extend(self.0);
        Ok(bytes)
    }
}
pub struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn take(&mut self, size: usize) -> Result<&'a [u8], Error> {
        if size > self.bytes.len() {
            return Err(protocol("incomplete helper answer"));
        }
        let (head, tail) = self.bytes.split_at(size);
        self.bytes = tail;
        Ok(head)
    }
    pub fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub fn int(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(
            self.take(4)?.try_into().map_err(|_| protocol("integer"))?,
        ))
    }
    pub fn long(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().map_err(|_| protocol("integer"))?,
        ))
    }
    pub fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let n = self.int()? as usize;
        self.take(n)
    }
    pub fn text(&mut self) -> Result<String, Error> {
        let s =
            std::str::from_utf8(self.bytes()?).map_err(|_| protocol("non UTF-8 helper text"))?;
        if s.contains('\0') {
            return Err(protocol("NUL in helper text"));
        }
        Ok(s.to_owned())
    }
    pub fn boolean(&mut self) -> Result<bool, Error> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(protocol("invalid boolean")),
        }
    }
    pub fn done(self) -> Result<(), Error> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(protocol("trailing helper answer"))
        }
    }
}
pub fn answer(bytes: &[u8], kind: u32) -> Result<Vec<u8>, Error> {
    if bytes.len() < 17 || &bytes[..4] != b"RH01" || bytes.len() > LIMIT + 16 {
        return Err(protocol("invalid helper frame"));
    }
    let mut r = Reader::new(&bytes[4..]);
    if r.int()? != kind | 0x80000000 {
        return Err(protocol("foreign helper operation"));
    }
    if r.long()? != (bytes.len() - 16) as u64 {
        return Err(protocol("invalid helper frame length"));
    }
    match r.byte()? {
        0 => Ok(bytes[17..].to_vec()),
        1 => {
            let code = r.text()?;
            let message = r.text()?;
            let retryable = r.boolean()?;
            r.done()?;
            let code = match code.as_str() {
                "PROTOCOL_ERROR" => "PROTOCOL_ERROR",
                "HELPER_INCOMPATIBLE" => "HELPER_INCOMPATIBLE",
                "STATE_UNAVAILABLE" => "STATE_UNAVAILABLE",
                "NAMESPACE_MISMATCH" => "NAMESPACE_MISMATCH",
                "REQUEST_EXPIRED" => "REQUEST_EXPIRED",
                "IDEMPOTENCY_CONFLICT" => "IDEMPOTENCY_CONFLICT",
                "RUN_NOT_FOUND" => "RUN_NOT_FOUND",
                "RUN_CONTROL_UNAVAILABLE" => "RUN_CONTROL_UNAVAILABLE",
                "OUTPUT_RANGE_UNAVAILABLE" => "OUTPUT_RANGE_UNAVAILABLE",
                "CONFIG_INVALID" => "CONFIG_INVALID",
                _ => return Err(protocol("unknown helper refusal")),
            };
            Err(Error {
                code,
                message,
                retryable,
            })
        }
        _ => Err(protocol("invalid helper result tag")),
    }
}
pub fn protocol(message: &str) -> Error {
    Error::new("PROTOCOL_ERROR", message)
}
