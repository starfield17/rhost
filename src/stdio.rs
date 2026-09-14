//! Honest access to the process's own standard output.
//!
//! `std::io::Stdout` deliberately maps `EBADF` to success, so that a GUI process
//! without a console can print harmlessly. That is exactly the wrong answer here:
//! a caller whose stdout is a read-only descriptor must learn the document was
//! lost, not receive a status that says it was delivered (EXEC-010).
//!
//! A duplicate of descriptor 1 shares the same open file description, so bytes
//! still land in the caller's file, pipe or terminal in order, but the duplicate
//! is written through `File`, where the kernel's answer is reported instead of
//! discarded.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;

/// A writer for descriptor 1 whose errors are real.
///
/// The descriptor is duplicated on the first write, so that a process which
/// cannot write to stdout still performs the operation it was asked to perform
/// and then reports the lost result, instead of refusing to start.
pub struct Stdout {
    file: Option<File>,
    failure: Option<(io::ErrorKind, String)>,
}

impl Stdout {
    pub fn open() -> Self {
        match io::stdout().as_fd().try_clone_to_owned() {
            Ok(duplicate) => Self {
                file: Some(File::from(duplicate)),
                failure: None,
            },
            Err(error) => Self {
                file: None,
                failure: Some((error.kind(), error.to_string())),
            },
        }
    }
}

impl Write for Stdout {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let Some(file) = self.file.as_mut() else {
            let (kind, message) = match &self.failure {
                Some((kind, message)) => (*kind, message.clone()),
                None => (io::ErrorKind::BrokenPipe, "stdout is closed".to_string()),
            };
            return Err(io::Error::new(kind, message));
        };
        file.write(data)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}
