use tokio::net::TcpListener;
use tokio::codec::{Decoder, Encoder};
use tokio::prelude::*;
use futures::channel::mpsc;
use bytes::BytesMut;
use std::io;
use std::net::SocketAddr;

const PING_PACKET: &'static str = "*1\r\n$4\r\nPING\r\n";
const PING_INLINE_PACKET: &'static str = "PING\r\n";
const PONG_PACKET: &'static str = "+PONG\r\n";
const BULK_PREFIX: u8 = '*' as u8;

// Only accepts Redis PING packet and only responds Redis PONG packet.
struct RedisPingPongCodec;

impl Decoder for RedisPingPongCodec {
    type Item = ();
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let prefix = match buf.get(0) {
            Some(c) => *c,
            None => return Ok(None),
        };
        let len = if prefix == BULK_PREFIX { PING_PACKET.len() } else { PING_INLINE_PACKET.len() };
        if buf.len() < len {
            return Ok(None)
        }
        let _data = buf.split_to(PING_PACKET.len());
        Ok(Some(()))
    }
}

impl Encoder for RedisPingPongCodec {
    type Item = ();
    type Error = io::Error;

    fn encode(&mut self, _item: Self::Item, buf: &mut BytesMut) -> Result<(), Self::Error> {
        buf.extend_from_slice(PONG_PACKET.as_bytes());
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:6379".parse::<SocketAddr>()?;
    let mut listener = TcpListener::bind(&addr).await?;
    loop {
        let (socket, _) = listener.accept().await?;

        tokio::spawn(async move {
            let (mut writer, mut reader) = RedisPingPongCodec.framed(socket).split();
            let (mut tx, mut rx) = mpsc::unbounded();

            let reader_handler = async move {
                while let Some(res) = reader.next().await {
                    if let Err(err) = res {
                        println!("reader error: {}", err);
                        break;
                    }
                    if let Err(err) = tx.send(()).await {
                        println!("tx error: {}", err);
                        break;
                    }
                }
                println!("reader closed");
            };

            let writer_handler = async move {
                while let Some(()) = rx.next().await {
                    if let Err(err) = writer.send(()).await {
                        println!("writer error: {}", err);
                    }
                }
                println!("writer closed");
            };

            tokio::spawn(writer_handler);
            tokio::spawn(reader_handler);
            println!("spawn new connection");
        });
    }
}
