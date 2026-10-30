//! A Herdr-shaped projection of shared layout, private view, and core terminal facts.
//! Field names follow Herdr's `ClientShellSnapshot` so the adapted chrome reads naturally.
use herdr_client_lib::{
    commands::{SPACE_LABEL, qualify},
    protocol::Terminal,
    view::{Attention, Identity, View},
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    Unknown,
}

impl Status {
    /// Maps a core OSC 7501 word; an idle or done word the viewer has not seen is Herdr's done.
    pub fn from_word(word: Option<&str>, unseen: bool) -> Self {
        match word {
            Some("working") => Self::Working,
            Some("blocked" | "error") => Self::Blocked,
            Some("idle" | "done") if unseen => Self::Done,
            Some("idle" | "done") => Self::Idle,
            _ => Self::Unknown,
        }
    }
    pub fn priority(self) -> u8 {
        match self {
            Self::Blocked => 4,
            Self::Done => 3,
            Self::Working => 2,
            Self::Idle => 1,
            Self::Unknown => 0,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }
}

/// Git facts for one folder on the space's machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitInfo {
    pub top: String,
    pub branch: Option<String>,
    pub common_dir: String,
    pub linked: bool,
    pub ahead_behind: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub key: String,
    pub is_linked_worktree: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub workspace_id: String,
    pub number: usize,
    pub label: String,
    pub custom_label: bool,
    pub cwd: String,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub git_ahead_behind: Option<(usize, usize)>,
    pub worktree: Option<Worktree>,
    pub focused: bool,
    pub agent_status: Status,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    pub number: usize,
    pub label: String,
    pub custom_label: bool,
    pub zoomed: bool,
    pub focused: bool,
    pub agent_status: Status,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub label: Option<String>,
    pub terminal: Option<String>,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    /// The pane showing it, or `@address` for a terminal without a pane.
    pub pane_id: String,
    pub address: String,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub workspace_label: String,
    pub agent: Option<String>,
    pub title: Option<String>,
    pub agent_status: Status,
    pub state_change_seq: u64,
    pub focused: bool,
}

/// A live terminal that no pane on this machine shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loose {
    pub address: String,
    pub label: String,
    pub cwd: String,
    pub agent_status: Status,
    pub reported: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub focused_workspace_id: Option<String>,
    pub focused_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub workspaces: Vec<Workspace>,
    pub tabs: Vec<Tab>,
    pub panes: Vec<Pane>,
    pub agents: Vec<Agent>,
    pub loose: Vec<Loose>,
}

impl Snapshot {
    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.workspace_id == id)
    }
    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.tab_id == id)
    }
    pub fn pane(&self, id: &str) -> Option<&Pane> {
        self.panes.iter().find(|p| p.pane_id == id)
    }
}

pub struct Sources<'a> {
    pub document: Option<&'a Value>,
    pub view: &'a View,
    pub machine: &'a str,
    pub terminals: &'a BTreeMap<Identity, Terminal>,
    pub attention: &'a Attention,
    pub sequences: &'a BTreeMap<Identity, u64>,
    pub git: &'a HashMap<String, GitInfo>,
    /// The address shown without a pane, when the viewer is previewing one.
    pub peek: Option<&'a str>,
}

/// The status word core reported for a terminal record.
pub fn word(record: &Terminal) -> Option<&str> {
    record.word()
}

/// Where a terminal's foreground program works now, else where it started.
pub fn cwd(record: &Terminal) -> Option<&str> {
    record.folder()
}

/// Whether a terminal belongs in the agents list: it runs and reports status through OSC 7501.
pub fn is_agent(record: &Terminal) -> bool {
    record.state != "finished" && (record.reported || record.status.is_some())
}

/// What a terminal's program calls itself: its reported app, name, or program.
fn program_label(record: &Terminal) -> Option<&str> {
    record
        .status
        .as_ref()
        .map(|s| s.app.as_str())
        .filter(|s| !s.is_empty())
        .or(record.name.as_deref())
        .or_else(|| {
            record
                .argv
                .first()
                .map(|p| p.rsplit('/').next().unwrap_or(p))
        })
        .filter(|s| !s.is_empty())
}

fn status_title(record: &Terminal) -> Option<&str> {
    record
        .status
        .as_ref()
        .map(|s| s.title.as_str())
        .filter(|s| !s.is_empty())
}

/// Herdr's automatic label for a folder: its last component.
pub fn folder_label(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return if path.is_empty() {
            "~".into()
        } else {
            "/".into()
        };
    }
    trimmed
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(trimmed)
        .into()
}

/// Every terminal block in a layout subtree, in tree order.
pub fn blocks(node: &Value) -> Vec<&Value> {
    if node["type"] == "block" {
        return vec![node];
    }
    node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(blocks)
        .collect()
}

fn name(node: &Value) -> Option<&str> {
    node["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub fn build(sources: &Sources<'_>) -> Snapshot {
    let records: HashMap<&str, (&Identity, &Terminal)> = sources
        .terminals
        .iter()
        .map(|(identity, record)| (record.address.as_str(), (identity, record)))
        .collect();
    let status_of = |address: &str| {
        records
            .get(address)
            .map(|(identity, record)| {
                let unseen = sources
                    .attention
                    .entries
                    .iter()
                    .any(|entry| &entry.identity == *identity && entry.unseen);
                Status::from_word(word(record), unseen)
            })
            .unwrap_or_default()
    };
    let view = sources.view;
    let peeking = sources.peek.is_some();
    let mut snapshot = Snapshot {
        focused_workspace_id: view.active_space.clone().filter(|_| !peeking),
        focused_tab_id: view.active_tab.clone().filter(|_| !peeking),
        focused_pane_id: view.focused.clone().filter(|_| !peeking),
        ..Snapshot::default()
    };
    let mut placed = HashMap::<String, (String, String, String)>::new();
    let spaces = sources
        .document
        .and_then(|doc| doc["spaces"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for (space_index, space) in spaces.iter().enumerate() {
        let Some(space_id) = space["id"].as_str() else {
            continue;
        };
        let tabs = space["tabs"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let first_cwd = tabs
            .iter()
            .flat_map(|tab| blocks(&tab["root"]))
            .filter_map(|block| block["content"]["terminal"].as_str())
            .find_map(|local| {
                records
                    .get(qualify(local, sources.machine).as_str())
                    .and_then(|(_, record)| cwd(record))
            });
        let cwd = first_cwd
            .or_else(|| space["folder"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_owned();
        let git = sources.git.get(&cwd);
        let custom = name(space);
        let label = custom
            .map(str::to_owned)
            .unwrap_or_else(|| folder_label(git.map_or(cwd.as_str(), |git| git.top.as_str())));
        let mut space_status = Status::Unknown;
        for (tab_index, tab) in tabs.iter().enumerate() {
            let Some(tab_id) = tab["id"].as_str() else {
                continue;
            };
            let mut tab_status = Status::Unknown;
            for block in blocks(&tab["root"]) {
                let Some(pane_id) = block["id"].as_str() else {
                    continue;
                };
                let terminal = block["content"]["terminal"]
                    .as_str()
                    .map(|local| qualify(local, sources.machine));
                if let Some(address) = &terminal {
                    let status = status_of(address);
                    if status.priority() > tab_status.priority() {
                        tab_status = status;
                    }
                    placed.insert(
                        address.clone(),
                        (pane_id.into(), space_id.into(), tab_id.into()),
                    );
                }
                let title = terminal
                    .as_deref()
                    .and_then(|address| records.get(address))
                    .and_then(|(_, record)| status_title(record));
                snapshot.panes.push(Pane {
                    pane_id: pane_id.into(),
                    workspace_id: space_id.into(),
                    tab_id: tab_id.into(),
                    label: name(block).or(title).map(str::to_owned),
                    terminal,
                    focused: snapshot.focused_pane_id.as_deref() == Some(pane_id),
                });
            }
            if tab_status.priority() > space_status.priority() {
                space_status = tab_status;
            }
            let custom = name(tab);
            snapshot.tabs.push(Tab {
                tab_id: tab_id.into(),
                workspace_id: space_id.into(),
                number: tab_index + 1,
                label: custom
                    .map(str::to_owned)
                    .unwrap_or_else(|| (tab_index + 1).to_string()),
                custom_label: custom.is_some(),
                zoomed: snapshot.focused_tab_id.as_deref() == Some(tab_id) && view.zoomed.is_some(),
                focused: snapshot.focused_tab_id.as_deref() == Some(tab_id),
                agent_status: tab_status,
            });
        }
        snapshot.workspaces.push(Workspace {
            workspace_id: space_id.into(),
            number: space_index + 1,
            label,
            custom_label: custom.is_some(),
            repository: git.map(|git| git.top.clone()),
            branch: git.and_then(|git| git.branch.clone()),
            git_ahead_behind: git.and_then(|git| git.ahead_behind),
            worktree: git.map(|git| Worktree {
                key: git.common_dir.clone(),
                is_linked_worktree: git.linked,
            }),
            cwd,
            focused: snapshot.focused_workspace_id.as_deref() == Some(space_id),
            agent_status: space_status,
        });
    }
    for (identity, record) in sources.terminals {
        let address = record.address.as_str();
        let place = placed.get(address);
        let unseen = sources
            .attention
            .entries
            .iter()
            .any(|entry| &entry.identity == identity && entry.unseen);
        if place.is_none() && record.is_live() {
            snapshot.loose.push(Loose {
                address: address.into(),
                label: program_label(record).unwrap_or("terminal").to_owned(),
                cwd: cwd(record).unwrap_or_default().to_owned(),
                agent_status: Status::from_word(word(record), unseen),
                reported: is_agent(record),
                focused: sources.peek == Some(address),
            });
        }
        if !is_agent(record) {
            continue;
        }
        let workspace_label = place
            .and_then(|(_, space, _)| snapshot.workspace(space))
            .map(|w| w.label.clone())
            .or_else(|| {
                let space = record.label(SPACE_LABEL)?;
                Some(
                    snapshot
                        .workspace(space)
                        .map_or_else(|| space.to_owned(), |w| w.label.clone()),
                )
            })
            .unwrap_or_else(|| address.to_owned());
        let agent = program_label(record).map(str::to_owned);
        let pane_id = place
            .map(|(pane, _, _)| pane.clone())
            .unwrap_or_else(|| format!("@{address}"));
        let focused = match sources.peek {
            Some(peek) => peek == address,
            None => snapshot.focused_pane_id.as_deref() == Some(pane_id.as_str()),
        };
        snapshot.agents.push(Agent {
            pane_id,
            address: address.into(),
            workspace_id: place.map(|(_, space, _)| space.clone()),
            tab_id: place.map(|(_, _, tab)| tab.clone()),
            workspace_label,
            agent,
            title: status_title(record).map(str::to_owned),
            agent_status: Status::from_word(word(record), unseen),
            state_change_seq: sources.sequences.get(identity).copied().unwrap_or(0),
            focused,
        });
    }
    let order = |agent: &Agent| {
        let workspace = agent
            .workspace_id
            .as_deref()
            .and_then(|id| {
                snapshot
                    .workspaces
                    .iter()
                    .position(|w| w.workspace_id == id)
            })
            .unwrap_or(usize::MAX);
        let pane = snapshot
            .panes
            .iter()
            .position(|p| p.pane_id == agent.pane_id)
            .unwrap_or(usize::MAX);
        (workspace, pane)
    };
    let mut agents = std::mem::take(&mut snapshot.agents);
    agents.sort_by_key(order);
    snapshot.agents = agents;
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity(local: &str) -> Identity {
        Identity {
            core: "c".into(),
            local: local.into(),
        }
    }

    #[test]
    fn projects_herdr_labels_status_and_unplaced_agents() {
        let doc = json!({"spaces":[
            {"id":"s1","name":"","folder":"/work/app","tabs":[
                {"id":"t1","name":"","root":{"type":"row","children":[
                    {"type":"block","id":"p1","content":{"terminal":"t1"}},
                    {"type":"block","id":"p2","name":"reviewer","content":{"terminal":"t2"}}]}},
                {"id":"t2","name":"logs","root":{"type":"block","id":"p3","content":{"terminal":"t3"}}}]},
            {"id":"s2","name":"Docs","folder":"/","tabs":[]}]});
        let terminals = BTreeMap::from([
            (
                identity("t1"),
                serde_json::from_value::<Terminal>(json!({"address":"t1","cwd":"/work/app/src","status":{"state":"working","app":"claude"},"reported":true})).unwrap(),
            ),
            (
                identity("t2"),
                serde_json::from_value::<Terminal>(json!({"address":"t2","status":{"state":"idle"},"reported":true})).unwrap(),
            ),
            (
                identity("t3"),
                serde_json::from_value::<Terminal>(json!({"address":"t3","status":null,"reported":false})).unwrap(),
            ),
            (
                identity("t8"),
                serde_json::from_value::<Terminal>(json!({"address":"t8","state":"finished","status":null,"reported":true,"labels":{"herdr.space":"s1"}})).unwrap(),
            ),
            (
                identity("t9"),
                serde_json::from_value::<Terminal>(json!({"address":"t9","state":"live","status":{"state":"blocked"},"reported":true,"labels":{"herdr.space":"s2"}})).unwrap(),
            ),
        ]);
        let mut attention = Attention::default();
        attention.observe(identity("t2"), Some("working".into()), false);
        attention.observe(identity("t2"), Some("idle".into()), false);
        let view = View {
            active_space: Some("s1".into()),
            active_tab: Some("t1".into()),
            focused: Some("p1".into()),
            zoomed: Some("p1".into()),
            history: vec![],
        };
        let git = HashMap::from([(
            "/work/app/src".to_owned(),
            GitInfo {
                top: "/work/app".into(),
                branch: Some("main".into()),
                common_dir: "/work/app/.git".into(),
                ..GitInfo::default()
            },
        )]);
        let snapshot = build(&Sources {
            document: Some(&doc),
            view: &view,
            machine: "",
            terminals: &terminals,
            attention: &attention,
            sequences: &BTreeMap::new(),
            git: &git,
            peek: None,
        });
        let space = &snapshot.workspaces[0];
        assert_eq!((space.label.as_str(), space.custom_label), ("app", false));
        assert_eq!(space.branch.as_deref(), Some("main"));
        assert_eq!(space.agent_status, Status::Done);
        assert_eq!(snapshot.workspaces[1].label, "Docs");
        assert_eq!(snapshot.tabs[0].label, "1");
        assert!(snapshot.tabs[0].zoomed && snapshot.tabs[0].focused);
        assert_eq!(snapshot.tabs[1].label, "logs");
        assert_eq!(snapshot.panes[1].label.as_deref(), Some("reviewer"));
        let agents: Vec<_> = snapshot
            .agents
            .iter()
            .map(|a| {
                (
                    a.pane_id.as_str(),
                    a.agent_status,
                    a.workspace_label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            agents,
            vec![
                ("p1", Status::Working, "app"),
                ("p2", Status::Done, "app"),
                ("@t9", Status::Blocked, "Docs"),
            ]
        );
        assert_eq!(snapshot.agents[0].agent.as_deref(), Some("claude"));
        let loose: Vec<_> = snapshot
            .loose
            .iter()
            .map(|l| (l.address.as_str(), l.reported))
            .collect();
        assert_eq!(loose, vec![("t9", true)]);
        assert!(snapshot.agents[0].focused);
    }

    #[test]
    fn folder_labels_use_the_last_component() {
        assert_eq!(folder_label("/home/me/project/"), "project");
        assert_eq!(folder_label("/"), "/");
        assert_eq!(folder_label(""), "~");
    }
}
