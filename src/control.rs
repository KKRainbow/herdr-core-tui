//! Per-instance local control, alive only for the viewer lifetime.
use crate::Result;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, oneshot},
};
const LIMIT: u64 = 65_536;
const TIME: Duration = Duration::from_secs(5);
pub struct Request {
    pub delivered: oneshot::Receiver<()>,
    pub command: String,
    pub args: Value,
    pub reply: oneshot::Sender<Value>,
}
pub struct Endpoint {
    pub requests: mpsc::Receiver<Request>,
    path: PathBuf,
    task: tokio::task::JoinHandle<()>,
}
fn hash(text: &str) -> u64 {
    text.bytes().fold(14695981039346656037u64, |hash, b| {
        (hash ^ u64::from(b)).wrapping_mul(1099511628211)
    })
}
pub fn directory() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "herder-{:x}",
                hash(&std::env::var("HOME").unwrap_or_default())
            ))
        })
        .join("herder-tui")
}
fn prefix(session: &str) -> String {
    format!("{:x}-", hash(session))
}
fn path(session: &str, instance: &str) -> Result<PathBuf> {
    if instance.is_empty()
        || instance.len() > 32
        || !instance
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Instance must contain 1–32 letters, digits, hyphens, or underscores".into());
    }
    Ok(directory().join(format!("{}{instance}.sock", prefix(session))))
}
impl Endpoint {
    pub fn bind(session: &str, instance: &str) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let directory = directory();
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let path = path(session, instance)?;
        if path.exists() {
            use std::os::unix::fs::FileTypeExt;
            if std::fs::symlink_metadata(&path)?.file_type().is_socket() {
                let socket = std::os::unix::net::UnixStream::connect(&path);
                if socket
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
                {
                    std::fs::remove_file(&path)?;
                }
            }
        }
        let listener = UnixListener::bind(&path).map_err(|e| {
            format!("Cannot bind instance {instance}: {e}. Choose a unique instance name.")
        })?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let (tx, requests) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            let capacity = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let Ok(permit) = capacity.clone().acquire_owned().await else {
                    break;
                };
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = tokio::time::timeout(TIME, serve(socket, tx)).await;
                });
            }
        });
        Ok(Self {
            requests,
            path,
            task,
        })
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}
async fn serve(socket: UnixStream, tx: mpsc::Sender<Request>) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let (read, mut write) = socket.into_split();
    let mut line = String::new();
    BufReader::new(read.take(LIMIT))
        .read_line(&mut line)
        .await?;
    if !line.ends_with('\n') {
        return Err("Incomplete or oversized request".into());
    }
    let request: Value = serde_json::from_str(&line)?;
    let command = request["command"]
        .as_str()
        .ok_or("command required")?
        .to_owned();
    let args = request.get("args").cloned().unwrap_or(json!({}));
    let (reply, receiver) = oneshot::channel();
    let (delivered, delivery) = oneshot::channel();
    tx.send(Request {
        delivered: delivery,
        command,
        args,
        reply,
    })
    .await?;
    let result = receiver.await?;
    write.write_all(format!("{result}\n").as_bytes()).await?;
    let _ = delivered.send(());
    Ok(())
}
pub async fn call(session: &str, instance: &str, command: &str, args: Value) -> Result<Value> {
    use tokio::io::AsyncReadExt;
    let path = path(session, instance)?;
    tokio::time::timeout(TIME, async {
        let mut socket = UnixStream::connect(path).await?;
        let request = format!("{}\n", json!({"command":command,"args":args}));
        if request.len() >= LIMIT as usize {
            return Err("Request too large".into());
        }
        socket.write_all(request.as_bytes()).await?;
        let mut line = String::new();
        BufReader::new(socket.take(LIMIT))
            .read_line(&mut line)
            .await?;
        if !line.ends_with('\n') {
            return Err("Incomplete response".into());
        }
        Ok(serde_json::from_str(&line)?)
    })
    .await
    .map_err(|_| "Viewer did not respond within five seconds")?
}
pub async fn instances(session: &str) -> Result<Value> {
    let prefix = prefix(session);
    let mut instances = vec![];
    match std::fs::read_dir(directory()) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(instance) = name
                    .strip_prefix(&prefix)
                    .and_then(|n| n.strip_suffix(".sock"))
                    && let Ok(state) = call(session, instance, "state", json!({})).await
                {
                    instances.push(json!({"instance":instance,"state":state}));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    };
    Ok(json!({"instances":instances}))
}
