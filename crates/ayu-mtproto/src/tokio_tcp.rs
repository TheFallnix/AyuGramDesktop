use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

pub const TCP_EVENT_CONNECTED: u8 = 1;
pub const TCP_EVENT_READY_READ: u8 = 2;
pub const TCP_EVENT_DISCONNECTED: u8 = 3;
pub const TCP_EVENT_ERROR: u8 = 4;

const MAX_READ_CHUNK: usize = 32 * 1024;
const MAX_READ_BUFFER_BYTES: usize = 512 * 1024;

type EventCallback = Arc<dyn Fn(u64, u8) + Send + Sync>;

struct SocketState {
    connected: AtomicBool,
    closed: AtomicBool,
    read_buf: Mutex<VecDeque<u8>>,
    write_tx: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    pending_writes: Mutex<Vec<Vec<u8>>>,
}

impl SocketState {
    fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            read_buf: Mutex::new(VecDeque::new()),
            write_tx: Mutex::new(None),
            pending_writes: Mutex::new(Vec::new()),
        }
    }
}

/// Shared Tokio-backed TCP connection pool for direct MTProto DC sockets.
pub struct TokioTcpManager {
    handle: Handle,
    next_id: AtomicU64,
    sockets: Mutex<HashMap<u64, Arc<SocketState>>>,
}

impl TokioTcpManager {
    pub fn new(handle: Handle) -> Self {
        Self {
            handle,
            next_id: AtomicU64::new(1),
            sockets: Mutex::new(HashMap::new()),
        }
    }

    pub fn connect(&self, host: String, port: u16, on_event: EventCallback) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let state = Arc::new(SocketState::new());
        if let Ok(mut map) = self.sockets.lock() {
            map.insert(id, Arc::clone(&state));
        }

        let task_state = Arc::clone(&state);
        self.handle.spawn(async move {
            let addr = format!("{host}:{port}");
            let stream = match TcpStream::connect(&addr).await {
                Ok(stream) => stream,
                Err(_) => {
                    if !task_state.closed.load(Ordering::Relaxed) {
                        on_event(id, TCP_EVENT_ERROR);
                    }
                    return;
                }
            };
            if task_state.closed.load(Ordering::Relaxed) {
                return;
            }
            let _ = stream.set_nodelay(true);
            let (mut reader, mut writer) = stream.into_split();
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

            if let Ok(mut slot) = task_state.write_tx.lock() {
                *slot = Some(tx.clone());
            }
            if let Ok(mut pending) = task_state.pending_writes.lock() {
                for chunk in pending.drain(..) {
                    let _ = tx.send(chunk);
                }
            }
            task_state.connected.store(true, Ordering::Release);
            on_event(id, TCP_EVENT_CONNECTED);

            let writer_state = Arc::clone(&task_state);
            let writer_cb = Arc::clone(&on_event);
            let write_task = tokio::spawn(async move {
                while let Some(chunk) = rx.recv().await {
                    if writer_state.closed.load(Ordering::Relaxed) {
                        break;
                    }
                    if writer.write_all(&chunk).await.is_err() {
                        if !writer_state.closed.load(Ordering::Relaxed) {
                            writer_state.connected.store(false, Ordering::Release);
                            writer_cb(id, TCP_EVENT_ERROR);
                        }
                        break;
                    }
                }
            });

            let mut buf = vec![0u8; MAX_READ_CHUNK];
            loop {
                if task_state.closed.load(Ordering::Relaxed) {
                    break;
                }
                let buffered = task_state
                    .read_buf
                    .lock()
                    .map(|q| q.len())
                    .unwrap_or(0);
                if buffered >= MAX_READ_BUFFER_BYTES {
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    continue;
                }
                match reader.read(&mut buf).await {
                    Ok(0) => {
                        task_state.connected.store(false, Ordering::Release);
                        if !task_state.closed.load(Ordering::Relaxed) {
                            on_event(id, TCP_EVENT_DISCONNECTED);
                        }
                        break;
                    }
                    Ok(n) => {
                        if let Ok(mut q) = task_state.read_buf.lock() {
                            q.extend(&buf[..n]);
                        }
                        if !task_state.closed.load(Ordering::Relaxed) {
                            on_event(id, TCP_EVENT_READY_READ);
                        }
                    }
                    Err(_) => {
                        task_state.connected.store(false, Ordering::Release);
                        if !task_state.closed.load(Ordering::Relaxed) {
                            on_event(id, TCP_EVENT_ERROR);
                        }
                        break;
                    }
                }
            }

            if let Ok(mut slot) = task_state.write_tx.lock() {
                *slot = None;
            }
            write_task.abort();
        });

        id
    }

    pub fn write(&self, id: u64, data: &[u8]) -> bool {
        let state = match self.sockets.lock().ok().and_then(|m| m.get(&id).cloned()) {
            Some(s) => s,
            None => return false,
        };
        if state.closed.load(Ordering::Relaxed) {
            return false;
        }
        if let Ok(slot) = state.write_tx.lock() {
            if let Some(tx) = slot.as_ref() {
                return tx.send(data.to_vec()).is_ok();
            }
        }
        if let Ok(mut pending) = state.pending_writes.lock() {
            pending.push(data.to_vec());
            return true;
        }
        false
    }

    pub fn read(&self, id: u64, max_len: usize) -> Vec<u8> {
        let state = match self.sockets.lock().ok().and_then(|m| m.get(&id).cloned()) {
            Some(s) => s,
            None => return Vec::new(),
        };
        if let Ok(mut q) = state.read_buf.lock() {
            let take = max_len.min(q.len());
            let mut out = Vec::with_capacity(take);
            for _ in 0..take {
                if let Some(b) = q.pop_front() {
                    out.push(b);
                }
            }
            if q.is_empty() && q.capacity() > 64 * 1024 {
                q.shrink_to_fit();
            }
            return out;
        }
        Vec::new()
    }

    pub fn has_bytes(&self, id: u64) -> bool {
        self.sockets
            .lock()
            .ok()
            .and_then(|m| m.get(&id).cloned())
            .and_then(|s| s.read_buf.lock().ok().map(|q| !q.is_empty()))
            .unwrap_or(false)
    }

    pub fn is_connected(&self, id: u64) -> bool {
        self.sockets
            .lock()
            .ok()
            .and_then(|m| m.get(&id).cloned())
            .map(|s| s.connected.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    pub fn close(&self, id: u64) {
        let removed = self.sockets.lock().ok().and_then(|mut m| m.remove(&id));
        if let Some(state) = removed {
            state.closed.store(true, Ordering::Release);
            state.connected.store(false, Ordering::Release);
            if let Ok(mut slot) = state.write_tx.lock() {
                *slot = None;
            }
            if let Ok(mut q) = state.read_buf.lock() {
                q.clear();
                q.shrink_to_fit();
            }
        }
    }
}
