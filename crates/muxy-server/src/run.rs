use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use muxy_protocol::transport::{BindError, Listener, StreamCancellation, UnixSocketListener};
use muxy_server::{Registry, ServerEvent, connection};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use crate::{args::Args, logging, settings_file};

const POLL: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
type Clients = Arc<Mutex<BTreeMap<u64, Client>>>;

struct Client {
    events: Option<Sender<ServerEvent>>,
    cancellation: Box<dyn StreamCancellation>,
}

struct Socket {
    listener: Arc<UnixSocketListener>,
    path: PathBuf,
    identity: Mutex<Option<(u64, u64)>>,
}

impl Socket {
    fn bind(path: PathBuf) -> Result<Self, BindError> {
        validate_socket(&path)?;
        let listener = Arc::new(UnixSocketListener::bind(&path)?);
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            listener,
            path,
            identity: Mutex::new(Some((metadata.dev(), metadata.ino()))),
        })
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.close();
    }
}

impl Socket {
    fn close(&self) {
        if let Err(error) = self.remove() {
            log::error!("socket cleanup failed: {error}");
        }
    }

    fn remove(&self) -> io::Result<()> {
        let mut owned = self.identity.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(identity) = owned.take() else {
            return Ok(());
        };
        let result = self.remove_owned(identity);
        self.listener.close();
        result
    }

    fn remove_owned(&self, identity: (u64, u64)) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let directory = fs::File::open(parent)?;
        directory.lock()?;
        self.listener.close();
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if (metadata.dev(), metadata.ino()) == identity => {
                fs::remove_file(&self.path)
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub(crate) fn run(args: &Args) -> io::Result<()> {
    let mut signals = Signals::new([SIGTERM, SIGINT])?;
    let socket = match Socket::bind(args.socket.clone()) {
        Ok(socket) => Arc::new(socket),
        Err(BindError::InUse) => {
            writeln!(
                io::stdout(),
                "muxy-server already running at {}",
                args.socket.display()
            )?;
            return Ok(());
        }
        Err(error) => return Err(io::Error::other(error)),
    };
    let settings = settings_file::load(&args.settings)?;
    logging::init(&args.log)?;
    log::info!("server started: socket={}", args.socket.display());
    let (sender, events) = mpsc::channel();
    let directory = args
        .socket
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("sessions");
    let legacy = crate::legacy::read(directory.parent().unwrap_or_else(|| Path::new(".")))?;
    let registry = bootstrap_registry(args, settings, sender, &directory, legacy)?;
    let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let requested_stop = Arc::clone(&stopping);
    let requested_socket = Arc::clone(&socket);
    let clients = Clients::default();
    let subscribers = Arc::clone(&clients);
    let sessions = Arc::clone(&registry);
    let broadcast = thread::Builder::new()
        .name("server-events".into())
        .spawn(move || {
            broadcast(&events, &subscribers, &sessions, || {
                requested_stop.store(true, std::sync::atomic::Ordering::Release);
                requested_socket.close();
            });
        })?;
    let signal_handle = signals.handle();
    let closing_socket = Arc::clone(&socket);
    let stopped = Arc::clone(&stopping);
    let signal = thread::Builder::new()
        .name("server-signals".into())
        .spawn(move || {
            if signals.forever().next().is_some() {
                stopped.store(true, std::sync::atomic::Ordering::Release);
                closing_socket.close();
            }
        });
    let mut workers = Vec::new();
    let result = match signal.as_ref() {
        Ok(_) => accept(&socket, &registry, &clients, &mut workers).or_else(|error| {
            if stopping.load(std::sync::atomic::Ordering::Acquire) {
                Ok(())
            } else {
                Err(error)
            }
        }),
        Err(error) => Err(io::Error::other(error.to_string())),
    };
    socket.close();
    registry.shutdown();
    let broadcast_result = broadcast
        .join()
        .map_err(|_| io::Error::other("event thread panicked"));
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    while workers.iter().any(|worker| !worker.is_finished()) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    for client in lock(&clients).values() {
        client.cancellation.cancel();
    }
    let mut worker_result = Ok(());
    for worker in workers {
        if worker.join().is_err() {
            worker_result = Err(io::Error::other("client thread panicked"));
        }
    }
    signal_handle.close();
    let signal_result = match signal {
        Ok(signal) => signal
            .join()
            .map_err(|_| io::Error::other("signal thread panicked")),
        Err(_) => Ok(()),
    };
    log::info!("server stopped");
    socket.remove()?;
    result
        .and(broadcast_result)
        .and(worker_result)
        .and(signal_result)
}

fn bootstrap_registry(
    args: &Args,
    settings: muxy_server::ServerSettings,
    sender: Sender<ServerEvent>,
    directory: &Path,
    legacy: muxy_server::LegacyImport,
) -> io::Result<Arc<Registry>> {
    let hooks =
        muxy_server::ShellIntegration::install(&directory.with_file_name("shell-integration"))?;
    let settings_path = args.settings.clone();
    Ok(Arc::new(
        Registry::persistent_with_import(settings, sender, directory, legacy)?
            .with_shell_integration(hooks)
            .with_settings_persistence(move |settings| {
                settings_file::save(&settings_path, settings)
            }),
    ))
}

fn accept(
    socket: &Socket,
    registry: &Arc<Registry>,
    clients: &Clients,
    workers: &mut Vec<JoinHandle<()>>,
) -> io::Result<()> {
    for id in 1..u64::MAX {
        let stream = socket.listener.accept()?;
        let cancellation = stream.cancellation()?;
        let (sender, events) = mpsc::channel();
        lock(clients).insert(
            id,
            Client {
                events: Some(sender),
                cancellation,
            },
        );
        let sessions = Arc::clone(registry);
        let connected = Arc::clone(clients);
        log::info!("client connected: {id}");
        let worker = thread::Builder::new()
            .name(format!("client-{id}"))
            .spawn(move || {
                if let Err(error) = connection::serve(stream, sessions, events) {
                    log::error!("client {id}: {error}");
                }
                lock(&connected).remove(&id);
                log::info!("client disconnected: {id}");
            });
        match worker {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                lock(clients).remove(&id);
                return Err(error);
            }
        }
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                workers
                    .swap_remove(index)
                    .join()
                    .map_err(|_| io::Error::other("client thread panicked"))?;
            } else {
                index += 1;
            }
        }
    }
    Err(io::Error::other("client IDs exhausted"))
}

fn broadcast(
    events: &Receiver<ServerEvent>,
    clients: &Clients,
    registry: &Registry,
    stop: impl Fn(),
) {
    loop {
        match events.recv_timeout(POLL) {
            Ok(event) => publish(&event, clients, &stop),
            Err(RecvTimeoutError::Timeout) if !registry.is_stopped() => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                for event in events.try_iter() {
                    publish(&event, clients, &stop);
                }
                break;
            }
        }
    }
    for client in lock(clients).values_mut() {
        client.events.take();
    }
}

fn publish(event: &ServerEvent, clients: &Clients, stop: &impl Fn()) {
    match event {
        ServerEvent::StopRequested => stop(),
        ServerEvent::RestartRequested => {
            for client in lock(clients).values() {
                if let Some(sender) = &client.events {
                    let _ = sender.send(event.clone());
                }
            }
            stop();
        }
        ServerEvent::SessionEnded { id, reason } => {
            log::info!("session ended: {} ({reason:?})", id.get());
            for client in lock(clients).values() {
                if let Some(sender) = &client.events {
                    let _ = sender.send(event.clone());
                }
            }
        }
    }
}

fn lock(clients: &Clients) -> MutexGuard<'_, BTreeMap<u64, Client>> {
    clients.lock().unwrap_or_else(PoisonError::into_inner)
}

fn validate_socket(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_encoded_bytes().len();
    let limit = if cfg!(target_os = "linux") { 108 } else { 104 };
    if bytes >= limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path is {bytes} bytes; platform limit is {limit} bytes including the terminating NUL: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_length_counts_bytes_and_reserves_the_terminator() {
        let limit = if cfg!(target_os = "linux") { 108 } else { 104 };
        assert!(validate_socket(Path::new(&"a".repeat(limit - 1))).is_ok());
        assert!(validate_socket(Path::new(&"a".repeat(limit))).is_err());
        assert!(validate_socket(Path::new(&"é".repeat(limit / 2))).is_err());
    }

    #[test]
    fn closed_socket_cleanup_does_not_remove_its_replacement() -> io::Result<()> {
        let directory =
            Path::new("/tmp").join(format!("muxy-socket-{}", muxy_protocol::OperationId::new()));
        fs::create_dir(&directory)?;
        let path = directory.join("server.sock");
        for _ in 0..32 {
            let previous = Socket::bind(path.clone()).map_err(io::Error::other)?;
            previous.listener.close();
            previous.remove()?;
            assert!(!path.exists());
            let replacement = Socket::bind(path.clone()).map_err(io::Error::other)?;
            previous.remove()?;
            drop(previous);
            assert!(std::os::unix::net::UnixStream::connect(&path).is_ok());
            drop(replacement);
        }
        fs::remove_dir(directory)
    }
}
