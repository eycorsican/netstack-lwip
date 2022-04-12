use std::{net::SocketAddr, pin::Pin, sync::Arc};

use futures::stream::Stream;
use futures::task::{Context, Poll};

use super::tcp_listener_impl::TcpListenerImpl;
use super::tcp_stream::TcpStream;
use super::LWIPMutex;

pub struct TcpListener {
    inner: Box<TcpListenerImpl>,
}

impl TcpListener {
    pub(crate) fn new(lwip_mutex: Arc<LWIPMutex>) -> Self {
        TcpListener {
            inner: TcpListenerImpl::new(lwip_mutex),
        }
    }
}

impl Stream for TcpListener {
    type Item = (TcpStream, SocketAddr, SocketAddr);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        Stream::poll_next(Pin::new(&mut self.inner), cx)
    }
}
