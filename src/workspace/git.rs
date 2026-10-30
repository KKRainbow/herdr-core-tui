use super::{GIT_REFRESH, GitInfo, TIME, git_info};
use futures_util::{FutureExt, StreamExt, future::LocalBoxFuture, stream::FuturesUnordered};
use herdr_client_lib::{client::Client, worktrees::Git};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Instant,
};

const CONCURRENCY: usize = 4;

struct Folder {
    revision: u64,
    checked: Option<Instant>,
    running: Option<u64>,
}

pub(super) struct Update {
    cwd: String,
    revision: u64,
    info: Option<GitInfo>,
}

#[derive(Default)]
pub(super) struct Poller {
    context: Option<(String, u64)>,
    revision: u64,
    folders: BTreeMap<String, Folder>,
    pending: FuturesUnordered<LocalBoxFuture<'static, Update>>,
}

impl Poller {
    pub fn invalidate(&mut self) {
        for folder in self.folders.values_mut() {
            self.revision += 1;
            folder.revision = self.revision;
            folder.checked = None;
        }
    }

    pub fn refresh(
        &mut self,
        machine: &str,
        client: &Client,
        folders: HashSet<String>,
        cache: &mut HashMap<String, GitInfo>,
    ) -> bool {
        let context = (machine.to_owned(), client.generation());
        let mut changed = false;
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.folders.clear();
            changed = !cache.is_empty();
            cache.clear();
        }
        self.folders.retain(|cwd, _| folders.contains(cwd));
        cache.retain(|cwd, _| {
            let keep = folders.contains(cwd);
            changed |= !keep;
            keep
        });
        for cwd in folders {
            self.folders.entry(cwd).or_insert_with(|| {
                self.revision += 1;
                Folder {
                    revision: self.revision,
                    checked: None,
                    running: None,
                }
            });
        }
        for (cwd, folder) in &mut self.folders {
            if self.pending.len() >= CONCURRENCY {
                break;
            }
            if folder.running.is_some()
                || folder.checked.is_some_and(|at| at.elapsed() < GIT_REFRESH)
            {
                continue;
            }
            let git = Git {
                client: client.clone(),
                machine: machine.into(),
                deadline: TIME,
            };
            let cwd = cwd.clone();
            let revision = folder.revision;
            folder.running = Some(revision);
            self.pending.push(
                async move {
                    let info = git_info(&git, &cwd).await;
                    Update {
                        cwd,
                        revision,
                        info,
                    }
                }
                .boxed_local(),
            );
        }
        changed
    }

    pub async fn next(&mut self) -> Update {
        match self.pending.next().await {
            Some(update) => update,
            None => std::future::pending().await,
        }
    }

    pub fn apply(&mut self, update: Update, cache: &mut HashMap<String, GitInfo>) -> bool {
        let Some(folder) = self.folders.get_mut(&update.cwd) else {
            return false;
        };
        if folder.running == Some(update.revision) {
            folder.running = None;
        }
        if folder.revision != update.revision {
            return false;
        }
        folder.checked = Some(Instant::now());
        if cache.get(&update.cwd) == update.info.as_ref() {
            return false;
        }
        match update.info {
            Some(info) => cache.insert(update.cwd, info),
            None => cache.remove(&update.cwd),
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn client() -> (Client, tokio::io::DuplexStream) {
        let (transport, mut server) = tokio::io::duplex(4096);
        let (client, ()) = tokio::join!(Client::connect(transport, TIME), async {
            while server.read_u8().await.unwrap() != b'\n' {}
            server
                .write_all(b"{\"id\":0,\"result\":{\"generation\":1,\"framed\":true}}\n")
                .await
                .unwrap();
        });
        (client.unwrap(), server)
    }

    fn update(poller: &Poller, branch: &str) -> Update {
        Update {
            cwd: "/repo".into(),
            revision: poller.folders["/repo"].revision,
            info: Some(GitInfo {
                branch: Some(branch.into()),
                ..GitInfo::default()
            }),
        }
    }

    #[tokio::test]
    async fn invalidation_rejects_old_metadata_and_allows_a_fresh_query() {
        let (client, _server) = client().await;
        let mut poller = Poller::default();
        let mut cache = HashMap::new();
        let folders = HashSet::from(["/repo".into()]);
        poller.refresh("", &client, folders.clone(), &mut cache);
        let old = update(&poller, "old");
        poller.invalidate();
        assert!(!poller.apply(old, &mut cache));
        assert!(cache.is_empty());
        poller.refresh("", &client, folders, &mut cache);
        assert_eq!(
            poller.folders["/repo"].running,
            Some(poller.folders["/repo"].revision)
        );
        assert!(poller.apply(update(&poller, "fresh"), &mut cache));
        assert_eq!(cache["/repo"].branch.as_deref(), Some("fresh"));
    }

    #[tokio::test]
    async fn removed_and_readded_folders_reject_old_results() {
        let (client, _server) = client().await;
        let mut poller = Poller::default();
        let mut cache = HashMap::new();
        let folders = HashSet::from(["/repo".into()]);
        poller.refresh("", &client, folders.clone(), &mut cache);
        let old = update(&poller, "old");
        poller.refresh("", &client, HashSet::new(), &mut cache);
        poller.refresh("", &client, folders, &mut cache);
        assert!(!poller.apply(old, &mut cache));
        assert!(cache.is_empty());
        assert!(poller.apply(update(&poller, "fresh"), &mut cache));
    }

    #[tokio::test]
    async fn machine_and_connection_changes_reject_old_results_and_clear_cache() {
        let (first, _server) = client().await;
        let (second, _other_server) = client().await;
        let mut poller = Poller::default();
        let mut cache = HashMap::new();
        let folders = HashSet::from(["/repo".into()]);
        poller.refresh("", &first, folders.clone(), &mut cache);
        assert!(poller.apply(update(&poller, "local"), &mut cache));
        let old = update(&poller, "old-local");
        assert!(poller.refresh("remote", &first, folders.clone(), &mut cache));
        assert!(!poller.apply(old, &mut cache));
        assert!(cache.is_empty());
        let old = update(&poller, "old-connection");
        poller.refresh("remote", &second, folders, &mut cache);
        assert!(!poller.apply(old, &mut cache));
        assert!(cache.is_empty());
    }
}
