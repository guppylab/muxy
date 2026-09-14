use std::io;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use muxy_protocol::wire::{Decoder, Encoder};
use muxy_protocol::{Message, ReplyBody, RequestBody};

use super::fixture::{Fixture, Result};

#[derive(Clone, Copy)]
#[repr(u8)]
pub(super) enum Point {
    CreateRequest = 1,
    CreateReply,
    DiscardRequest,
    DiscardReply,
    ListReply,
    ResizeReply,
}

#[derive(Default)]
struct Gate {
    point: AtomicU8,
    reached: AtomicBool,
    released: AtomicBool,
    stop: AtomicBool,
    sockets: Mutex<Option<(UnixStream, UnixStream)>>,
}

impl Gate {
    fn intercepts(&self, message: &Message) -> bool {
        let point = match message {
            Message::Request {
                body: RequestBody::CreateSession { .. },
                ..
            } => Point::CreateRequest,
            Message::Reply {
                body: ReplyBody::SessionCreated(_),
                ..
            } => Point::CreateReply,
            Message::Request {
                body: RequestBody::CloseSession { .. },
                ..
            } => Point::DiscardRequest,
            Message::Reply {
                body: ReplyBody::SessionClosed,
                ..
            } => Point::DiscardReply,
            Message::Reply {
                body: ReplyBody::ProjectSessions(_),
                ..
            } => Point::ListReply,
            Message::Reply {
                body: ReplyBody::Resized,
                ..
            } => Point::ResizeReply,
            _ => return false,
        };
        self.point
            .compare_exchange(point as u8, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn disconnect(&self) {
        if let Some((client, server)) = &*self
            .sockets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            let _ = client.shutdown(Shutdown::Both);
            let _ = server.shutdown(Shutdown::Both);
        }
    }
}

pub(super) struct Proxy {
    gate: Arc<Gate>,
    thread: Option<JoinHandle<io::Result<()>>>,
    server: Child,
}

impl Proxy {
    pub(super) fn new(fixture: &Fixture) -> Result<Self> {
        let actual = fixture.directory.path().join("actual.sock");
        let server =
            std::process::Command::new(super::support::binary().with_file_name("muxy-server"))
                .envs(fixture.environment())
                .arg("--socket")
                .arg(&actual)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !actual.exists() {
            if Instant::now() >= deadline {
                return Err("Test server did not start".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let listener = UnixListener::bind(fixture.directory.path().join("server.sock"))?;
        listener.set_nonblocking(true)?;
        let gate = Arc::new(Gate::default());
        let relay = Arc::clone(&gate);
        let thread = thread::spawn(move || -> io::Result<()> {
            while !relay.stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((client, _)) => {
                        client.set_nonblocking(false)?;
                        let server = UnixStream::connect(&actual)?;
                        *relay
                            .sockets
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some((client.try_clone()?, server.try_clone()?));
                        let up_gate = Arc::clone(&relay);
                        let upstream = client.try_clone()?;
                        let destination = server.try_clone()?;
                        let up = thread::spawn(move || forward(upstream, destination, &up_gate));
                        forward(server, client, &relay);
                        relay.disconnect();
                        up.join()
                            .map_err(|_| io::Error::other("upstream proxy thread panicked"))?;
                        *relay
                            .sockets
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        });
        Ok(Self {
            gate,
            thread: Some(thread),
            server,
        })
    }

    pub(super) fn arm(&self, point: Point) {
        self.gate.reached.store(false, Ordering::Release);
        self.gate.released.store(false, Ordering::Release);
        self.gate.point.store(point as u8, Ordering::Release);
    }

    pub(super) fn reached(&self) -> bool {
        self.gate.reached.load(Ordering::Acquire)
    }

    pub(super) fn release(&self) {
        self.gate.released.store(true, Ordering::Release);
    }

    pub(super) fn disconnect(&self) {
        self.gate.disconnect();
        self.release();
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.gate.stop.store(true, Ordering::Release);
        self.disconnect();
        if let Some(thread) = self.thread.take() {
            assert!(matches!(thread.join(), Ok(Ok(()))), "proxy thread failed");
        }
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

fn forward(source: UnixStream, destination: UnixStream, gate: &Arc<Gate>) {
    let mut decoder = Decoder::new(source);
    let writer = Arc::new(Mutex::new(Encoder::new(destination)));
    let mut held = None;
    while let Ok((channel, message)) = decoder.next() {
        if gate.intercepts(&message) {
            let gate = Arc::clone(gate);
            let writer = Arc::clone(&writer);
            assert!(
                held.is_none(),
                "Only one held message per connection direction"
            );
            held = Some(thread::spawn(move || {
                gate.reached.store(true, Ordering::Release);
                while !gate.released.load(Ordering::Acquire) && !gate.stop.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(5));
                }
                if !gate.stop.load(Ordering::Acquire) {
                    let _ = writer
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .send(channel, &message);
                }
            }));
        } else if writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(channel, &message)
            .is_err()
        {
            break;
        }
    }
    gate.disconnect();
    if let Some(held) = held {
        // A disconnected client cannot consume a held reply.
        gate.released.store(true, Ordering::Release);
        assert!(held.join().is_ok(), "held message thread failed");
    }
}
