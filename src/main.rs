#[macro_use]
extern crate clap;
use clap::{Arg, App};
use tokio::net::TcpListener;
use tokio::codec::{Decoder, Encoder};
use tokio::prelude::*;
use futures::channel::mpsc;
use futures::{stream, Stream};
use futures_timer::Delay;
use bytes::BytesMut;
use std::io;
use std::net::SocketAddr;
use tokio_batch::{ChunksTimeout, StatsStrategy, FlushEvent};
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{Ordering, AtomicUsize};

const PING_PACKET: &'static str = "*1\r\n$4\r\nPING\r\n";
const PING_INLINE_PACKET: &'static str = "PING\r\n";
const PONG_PACKET: &'static str = "+PONG\r\n";
const BULK_PREFIX: u8 = '*' as u8;

#[derive(Debug, Clone, Copy, PartialEq)]
enum BatchMode {
    NoBatch,
    MaxTimer,
    MinMaxTimer,
}

// Only accepts Redis PING packet and only responds Redis PONG packet.
struct RedisPingPongCodec;

impl Decoder for RedisPingPongCodec {
    type Item = ();
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // redis-benchmark uses two different formats of PING.
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

struct Stats {
    full: AtomicUsize,
    min: AtomicUsize,
    max: AtomicUsize,
    flush_count: AtomicUsize,
    flush_size: AtomicUsize,
}

impl Stats {
    fn new() -> Self {
        Self {
            full: AtomicUsize::new(0),
            min: AtomicUsize::new(0),
            max: AtomicUsize::new(0),
            flush_count: AtomicUsize::new(0),
            flush_size: AtomicUsize::new(0),
        }
    }

    fn log(&self) {
        let flush_size = self.flush_size.load(Ordering::SeqCst) as i32;
        let flush_count = self.flush_count.load(Ordering::SeqCst) as i32;
        let full = self.full.load(Ordering::SeqCst);
        let min = self.min.load(Ordering::SeqCst);
        let max = self.max.load(Ordering::SeqCst);
        let sum: f64 = flush_size.into();
        let count: f64 = flush_count.into();
        let aver: f64 = if count == 0.0 { 0.0 } else { sum / count };
        println!("average flush size: {} full: {} min: {} max: {}", aver, full, min, max);
    }
}

struct LogStats {
    stats: Arc<Stats>,
}

impl LogStats {
    fn new(stats: Arc<Stats>) -> Self {
        Self { stats }
    }
}

impl StatsStrategy for LogStats {
    fn add(&mut self, event: FlushEvent, batch_size: usize) {
        match event {
            FlushEvent::Full => self.stats.full.fetch_add(1, Ordering::SeqCst),
            FlushEvent::MinTimeoutTimer => self.stats.min.fetch_add(1, Ordering::SeqCst),
            FlushEvent::MaxTimeoutTimer => self.stats.max.fetch_add(1, Ordering::SeqCst),
        };
        self.stats.flush_count.fetch_add(1, Ordering::SeqCst);
        self.stats.flush_size.fetch_add(batch_size, Ordering::SeqCst);
    }
}

fn wrap_stream<S>(s: S, mode: BatchMode, stats: Arc<Stats>, min_timeout: u64, max_timeout: u64) -> Box<dyn Stream<Item = Vec<()>> + Send + 'static + Unpin>
    where S: Stream<Item = ()> + Send + 'static + Unpin
{
    match mode {
        BatchMode::NoBatch => Box::new(s.map(|item| vec![item])),
        BatchMode::MaxTimer => {
            Box::new(
                ChunksTimeout::with_stats(
                    s,
                    10,
                    None,
                    Duration::from_micros(max_timeout),
                    LogStats::new(stats),
                )
            )
        },
        BatchMode::MinMaxTimer => {
            Box::new(
                ChunksTimeout::with_stats(
                    s,
                    10,
                    Some(Duration::from_micros(min_timeout)),
                    Duration::from_micros(max_timeout),
                    LogStats::new(stats),
                )
            )
        }
    }
}

#[tokio::main(single_thread)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = App::new("redis-ping-pong-server")
        .version("1.0")
        .author("doyoubi <doyoubihgx@gmail.com>")
        .about("Benchmarking tokio-batch")
        .arg(Arg::with_name("mode")
            .short("m")
            .long("mode")
            .value_name("MODE")
            .help("[nobatch|max|minmax]")
            .takes_value(true))
        .arg(Arg::with_name("min")
            .long("min")
            .value_name("MIN")
            .help("minimum timeout, only works for minmax mode")
            .takes_value(true))
        .arg(Arg::with_name("max")
            .long("max")
            .value_name("MAX")
            .help("maximum timeout, works for max and minmax mode")
            .takes_value(true))
        .get_matches();
    let mode = match matches.value_of("mode").unwrap_or("nobatch") {
        "max" => BatchMode::MaxTimer,
        "minmax" => BatchMode::MinMaxTimer,
        _ => BatchMode::NoBatch,
    };
    let min_timeout = value_t!(matches.value_of("min"), u64).unwrap_or(10);
    let max_timeout = value_t!(matches.value_of("max"), u64).unwrap_or(500);
    println!("mode: {:?} min timeout: {} max timeout: {}", mode, min_timeout, max_timeout);

    let stats = Arc::new(Stats::new());
    let stats_clone = stats.clone();

    if mode != BatchMode::NoBatch {
        tokio::spawn(async move {
            loop {
                Delay::new(Duration::from_secs(10)).await;
                stats_clone.log();
            }
        });
    }

    let addr = "127.0.0.1:6379".parse::<SocketAddr>()?;
    let mut listener = TcpListener::bind(&addr).await?;
    loop {
        let stats_clone = stats.clone();
        let (socket, _) = listener.accept().await?;

        tokio::spawn(async move {
            let (mut writer, mut reader) = RedisPingPongCodec.framed(socket).split();
            let (mut tx, rx) = mpsc::unbounded();

            let mut rx = wrap_stream(rx, mode, stats_clone, min_timeout, max_timeout);

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
                while let Some(reqs) = rx.next().await {
                    let mut batch = stream::iter(reqs.into_iter());
                    if let Err(err) = writer.send_all(&mut batch).await {
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
