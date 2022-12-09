use std::{io, pin::Pin};

use futures::sink::Sink;
use futures::stream::Stream;
use futures::task::{Context, Poll};

use super::stack_impl::NetStackImpl;
use super::tcp_listener::TcpListener;
use super::udp::UdpSocket;

pub struct NetStack(Box<NetStackImpl>);

impl NetStack {
    pub fn new() -> (Self, TcpListener, Box<UdpSocket>) {
        (
            NetStack(NetStackImpl::new(512)),
            TcpListener::new(),
            UdpSocket::new(64),
        )
    }

    pub fn with_buffer_size(
        stack_buffer_size: usize,
        udp_buffer_size: usize,
    ) -> (Self, TcpListener, Box<UdpSocket>) {
        (
            NetStack(NetStackImpl::new(stack_buffer_size)),
            TcpListener::new(),
            UdpSocket::new(udp_buffer_size),
        )
    }
}

impl Stream for NetStack {
    type Item = io::Result<Vec<u8>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}

impl Sink<Vec<u8>> for NetStack {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        Pin::new(&mut self.0).start_send(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_close(cx)
    }
}
