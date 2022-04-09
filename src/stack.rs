use std::{io, pin::Pin, sync::Arc};

use futures::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::stack_impl::NetStackImpl;
use super::LWIPMutex;

pub struct NetStack(Box<NetStackImpl>);

impl NetStack {
    pub fn new(lwip_mutex: Arc<LWIPMutex>) -> Self {
        NetStack(NetStackImpl::new(lwip_mutex))
    }
}

impl AsyncRead for NetStack {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.0), cx, buf)
    }
}

impl AsyncWrite for NetStack {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.0), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.0), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.0), cx)
    }
}
