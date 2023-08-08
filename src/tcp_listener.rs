use std::{net::SocketAddr, pin::Pin};

use futures::stream::Stream;
use futures::task::{Context, Poll};

use super::tcp_listener_impl::TcpListenerImpl;
use super::tcp_stream::TcpStream;
use crate::Error;

pub struct TcpListener {
    inner: Box<TcpListenerImpl>,
}

impl TcpListener {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(TcpListener {
            inner: TcpListenerImpl::new()?,
        })
    }
}

impl Stream for TcpListener {
    type Item = (TcpStream, SocketAddr, SocketAddr);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Stream::poll_next(Pin::new(&mut self.inner), cx)
    }
}
