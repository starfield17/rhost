//! Observe transport closure immediately; SDK framing remains untouched.
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};
pub struct Input<R> {
    reader: R,
    stopped: Arc<AtomicBool>,
}
impl<R> Input<R> {
    pub fn new(reader: R, stopped: Arc<AtomicBool>) -> Self {
        Self { reader, stopped }
    }
}
impl<R: AsyncRead + Unpin> AsyncRead for Input<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let remaining = buf.remaining();
        let result = Pin::new(&mut this.reader).poll_read(cx, buf);
        if matches!(&result, Poll::Ready(Err(_)))
            || (matches!(&result, Poll::Ready(Ok(())))
                && remaining > 0
                && before == buf.filled().len())
        {
            this.stopped.store(true, Ordering::SeqCst);
        }
        result
    }
}
impl<R> Drop for Input<R> {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}
