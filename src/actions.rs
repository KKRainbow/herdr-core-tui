use crate::Result;
use herdr_client_lib::{
    commands::{CloseAssessment, Launch, Placement},
    document::Axis,
    sync::{Session, WriteOutcome},
    view::View,
    worktrees::{CreatePlan, Git, RemovalRisk},
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub fn id(prefix: &str) -> String {
    format!(
        "{prefix}{:x}{:x}{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn text<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args[name].as_str()
}
fn required<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    text(args, name)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{name} is required").into())
}
fn written(outcome: WriteOutcome) -> Result<Value> {
    match outcome {
        WriteOutcome::Written => Ok(json!({"ok":true})),
        WriteOutcome::Conflict => {
            Err("The workspace changed. It was reloaded; review and try again.".into())
        }
        WriteOutcome::Unavailable(e) => Err(e.into()),
    }
}
fn target(args: &Value, view: &View, kind: &str) -> Result<String> {
    text(args, "id")
        .map(str::to_owned)
        .or_else(|| match kind {
            "space" => view.active_space.clone(),
            "tab" => view.active_tab.clone(),
            _ => view.focused.clone(),
        })
        .ok_or_else(|| "Select a target first".into())
}
fn layout_value(layout: &WriteOutcome) -> Value {
    match layout {
        WriteOutcome::Written => json!({"status":"written"}),
        WriteOutcome::Conflict => json!({"status":"conflict"}),
        WriteOutcome::Unavailable(error) => json!({"status":"unavailable","error":error}),
    }
}
fn creation_value(result: &herdr_client_lib::commands::Creation) -> Value {
    json!({"ok":matches!(result.layout,WriteOutcome::Written),"terminal":result.terminal,"correlation":result.correlation,"layout":layout_value(&result.layout)})
}
fn change_value(result: herdr_client_lib::commands::ChangeOutcome) -> Value {
    json!({"ok":result.failure.is_none()&&matches!(result.layout,Some(WriteOutcome::Written)),"terminal":result.terminal,"effects":result.effects.iter().map(|effect|json!({"kind":format!("{:?}",effect.kind),"result":effect.result})).collect::<Vec<_>>(),"failure":result.failure.as_ref().map(|failure|json!({"kind":format!("{:?}",failure.kind),"error":failure.error.to_string()})),"layout":result.layout.as_ref().map(layout_value)})
}
fn risk_word(risk: RemovalRisk) -> &'static str {
    match risk {
        RemovalRisk::Dirty => "dirty or untracked files",
        RemovalRisk::UnreferencedCommits => "commits no branch holds",
        RemovalRisk::UnpushedSubmoduleCommits => "unpushed submodule commits",
        RemovalRisk::Locked => "a locked checkout",
        RemovalRisk::Prunable => "a missing checkout folder",
        RemovalRisk::Bare => "a bare repository",
    }
}
fn git_value(output: &herdr_client_lib::worktrees::Output) -> Value {
    json!({"code":output.code,"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
}
#[cfg(test)]
use herdr_client_lib::commands::launch_source;
fn focus_created(view: &mut View, document: &herdr_client_lib::document::Document, pane: &str) {
    if view.focus(document, pane) {
        view.zoom(None);
    }
}
pub const HELP: &str = "space.new / tab.new / pane.split • name, folder, argv\nspace.rename / tab.rename / pane.rename • id, name\nspace.move / tab.move • id, index    space.move • ids, before\npane.move • id, destination, index\npane.swap • id, other    pane.resize • id, weight | id, next, share\nspace.close / tab.close / pane.close • id, confirm\nterminal.keep • terminal    terminal.park • id\nworktree.list / worktree.branches • repository\nworktree.new • repository, branch, path, new_branch\nworktree.open • path    worktree.remove • repository, path, id, confirm, force\nMachine, focus, zoom, copy and search are private viewer actions.";

/// Moves spaces together, in the given order, to just before `before` (or to the end).
fn move_spaces(
    document: &mut herdr_client_lib::document::Document,
    ids: &[String],
    before: Option<&str>,
) -> std::result::Result<(), herdr_client_lib::document::Error> {
    use herdr_client_lib::document::Error;
    let order: Vec<String> = document.value()["spaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|space| space["id"].as_str().map(str::to_owned))
        .collect();
    if ids.is_empty() || ids.iter().any(|id| !order.contains(id)) {
        return Err(Error::InvalidTarget);
    }
    let mut rest: Vec<String> = order.into_iter().filter(|id| !ids.contains(id)).collect();
    let at = match before {
        Some(target) => rest
            .iter()
            .position(|id| id == target)
            .ok_or(Error::InvalidTarget)?,
        None => rest.len(),
    };
    rest.splice(at..at, ids.iter().cloned());
    for (index, id) in rest.iter().enumerate() {
        document.reorder(id, index)?;
    }
    Ok(())
}

fn close_review(assessment: &CloseAssessment) -> Value {
    json!({"confirmation":"Closing ends these terminals and their dependent children","target":assessment.target,"version":assessment.version,"terminals":assessment.terminals.iter().map(|t|json!({"address":t.address,"pid":t.pid,"parent":t.parent,"labels":t.labels,"state":t.state})).collect::<Vec<_>>(),"protected":assessment.protected})
}

async fn close_group(session: &mut Session, machine: &str, args: &Value) -> Result<Value> {
    let ids: Vec<String> = serde_json::from_value(args["ids"].clone())?;
    if ids.is_empty() {
        return Err("Select workspaces to close".into());
    }
    let mut assessments = Vec::new();
    for id in ids {
        assessments.push(session.assess_close(machine, &id).await?);
    }
    let review = json!({"machine":machine,"targets":assessments.iter().map(close_review).collect::<Vec<_>>()});
    if args["confirm"] != true {
        return Ok(review);
    }
    if args.get("expected") != Some(&review)
        || assessments
            .iter()
            .any(|a| !a.protected.is_empty() || a.version != assessments[0].version)
    {
        return Err("The close target changed. Review it again.".into());
    }
    let mut version = assessments[0].version;
    for mut assessment in assessments {
        // Carry only our successful layout writes forward; keep the reviewed terminal facts.
        assessment.version = version;
        let result = session.close(assessment).await?;
        written(result.layout)?;
        version = session.documents[machine].version;
    }
    Ok(json!({"ok":true}))
}

pub async fn execute(
    session: &mut Session,
    machine: &str,
    view: &mut View,
    command: &str,
    args: Value,
) -> Result<Value> {
    if command == "help" {
        return Ok(json!({"help":HELP}));
    }
    if command == "space.close.group" {
        return close_group(session, machine, &args).await;
    }
    let kind = command.split('.').next().unwrap_or("");
    if matches!(command, "space.rename" | "tab.rename" | "pane.rename") {
        let id = target(&args, view, kind)?;
        let name = text(&args, "name").ok_or("name is required")?;
        return written(session.edit(machine, |d| d.rename(&id, name)).await?);
    }
    if command == "space.move" && args.get("ids").is_some() {
        let ids: Vec<String> = serde_json::from_value(args["ids"].clone())?;
        let before = text(&args, "before").map(str::to_owned);
        return written(
            session
                .edit(machine, |d| move_spaces(d, &ids, before.as_deref()))
                .await?,
        );
    }
    if matches!(command, "space.move" | "tab.move") {
        let id = target(&args, view, kind)?;
        let index = args["index"].as_u64().ok_or("index is required")? as usize;
        return written(session.edit(machine, |d| d.reorder(&id, index)).await?);
    }
    if matches!(command, "space.close" | "tab.close" | "pane.close") {
        let id = target(&args, view, kind)?;
        let assessment = session.assess_close(machine, &id).await?;
        let review = close_review(&assessment);
        if args["confirm"] != true {
            return Ok(review);
        }
        if args
            .get("expected")
            .is_some_and(|expected| expected != &review)
        {
            return Err("The close target changed. Review it again.".into());
        }
        let result = session.close(assessment).await?;
        return Ok(
            json!({"ok":matches!(result.layout,WriteOutcome::Written),"layout":layout_value(&result.layout),"observed":result.observed}),
        );
    }
    if command == "pane.resize" {
        let id = target(&args, view, "pane")?;
        if let Some(next) = text(&args, "next") {
            let share = args["share"].as_f64().ok_or("share is required")?;
            return written(
                session
                    .edit(machine, |d| d.resize_between(&id, next, share))
                    .await?,
            );
        }
        let weight = args["weight"].as_f64().ok_or("weight is required")?;
        return written(session.edit(machine, |d| d.resize(&id, weight)).await?);
    }
    if command == "pane.swap" {
        let id = target(&args, view, "pane")?;
        let other = required(&args, "other")?;
        return written(session.edit(machine, |d| d.swap(&id, other)).await?);
    }
    if command == "pane.move" {
        let id = target(&args, view, "pane")?;
        let dest = required(&args, "destination")?;
        return Ok(change_value(
            session
                .move_pane(
                    machine,
                    &id,
                    dest,
                    args["index"].as_u64().unwrap_or(0) as usize,
                )
                .await?,
        ));
    }
    if command == "terminal.park" {
        let id = target(&args, view, "pane")?;
        return Ok(change_value(session.park(machine, &id).await?));
    }
    let document = session.document(machine)?;
    let pane = id("p");
    let placement = match command {
        "space.new" | "worktree.new" | "worktree.open" => Some(Placement::Space {
            id: id("sp"),
            name: text(&args, "name").unwrap_or("").into(),
            tab: id("tb"),
            pane: pane.clone(),
        }),
        "tab.new" | "terminal.keep" => {
            let space = text(&args, "space")
                .map(str::to_owned)
                .or_else(|| view.active_space.clone())
                .ok_or("Select a space first")?;
            // New tabs go after the viewer's selected tab in that space, as in the app.
            let after = view.spaces.get(&space).cloned();
            Some(Placement::Tab {
                space,
                after,
                id: id("tb"),
                name: text(&args, "name").unwrap_or("").into(),
                pane: pane.clone(),
            })
        }
        "pane.split" => {
            let target = target(&args, view, "pane")?;
            let (space, tab) = document
                .location(&target)
                .ok_or("Pane is no longer available")?;
            Some(Placement::Split {
                space,
                tab,
                target,
                container: id("s"),
                pane: pane.clone(),
                axis: if args["axis"] == "column" {
                    Axis::Column
                } else {
                    Axis::Row
                },
            })
        }
        _ => None,
    };
    if command == "terminal.keep" {
        let outcome = session
            .adopt(machine, placement.unwrap(), required(&args, "terminal")?)
            .await?;
        if let Ok(doc) = session.document(machine) {
            focus_created(view, &doc, &pane);
        }
        return Ok(change_value(outcome));
    }
    if command.starts_with("worktree.") {
        let git = Git {
            client: session.client.clone(),
            machine: machine.into(),
            deadline: session.deadline,
        };
        match command {
            "worktree.list" => {
                return Ok(
                    json!({"worktrees":git.list(required(&args,"repository")?).await?.iter().map(|w|json!({"path":w.path,"branch":w.branch,"locked":w.locked})).collect::<Vec<_>>()}),
                );
            }
            "worktree.branches" => {
                return Ok(json!({"branches":git.branches(required(&args,"repository")?).await?}));
            }
            "worktree.open" => {
                let result = git
                    .open(
                        session,
                        required(&args, "path")?,
                        placement.unwrap(),
                        id("op"),
                    )
                    .await?;
                if let Ok(doc) = session.document(machine) {
                    focus_created(view, &doc, &pane);
                }
                return Ok(creation_value(&result));
            }
            "worktree.new" => {
                let repository = required(&args, "repository")?;
                let branch = required(&args, "branch")?;
                let path = match text(&args, "path") {
                    Some(path) => path.to_owned(),
                    None => {
                        let directory = crate::preferences::load()
                            .ok()
                            .and_then(|p| p.worktrees.directory);
                        git.default_path_in(repository, branch, directory.as_deref())
                            .await?
                    }
                };
                let result = git
                    .create_space(
                        session,
                        CreatePlan {
                            repository: repository.into(),
                            path,
                            branch: branch.into(),
                            new_branch: args["new_branch"] != false,
                            placement: placement.unwrap(),
                            correlation: id("op"),
                        },
                    )
                    .await?;
                if let Ok(doc) = session.document(machine) {
                    focus_created(view, &doc, &pane);
                }
                let workspace = match &result.workspace {
                    Ok(created) => creation_value(created),
                    Err(error) => json!({"ok":false,"error":error.to_string()}),
                };
                return Ok(
                    json!({"ok":result.git.code==Some(0)&&workspace["ok"]==true,"git":git_value(&result.git),"workspace":workspace}),
                );
            }
            "worktree.remove" => {
                let assessment = git
                    .assess_remove(required(&args, "repository")?, required(&args, "path")?)
                    .await?;
                let risks = assessment
                    .risks()
                    .iter()
                    .map(|risk| risk_word(*risk))
                    .collect::<Vec<_>>();
                if !assessment.allowed(true) {
                    return Err(format!("Checkout cannot be removed: {}", risks.join(", ")).into());
                }
                let force = args["force"] == true;
                let close = session
                    .assess_close(machine, &target(&args, view, "space")?)
                    .await?;
                let review = json!({"confirmation":"Remove this checkout and close its workspace","path":assessment.checkout.path,"version":close.version,"assessment":format!("{assessment:?}"),"risks":risks,"terminals":close.terminals.iter().map(|t|json!({"address":t.address,"pid":t.pid})).collect::<Vec<_>>()});
                if args["confirm"] != true {
                    return Ok(review);
                }
                if args
                    .get("expected")
                    .is_some_and(|expected| expected != &review)
                {
                    return Err("Removal conditions changed. Review the checkout again.".into());
                }
                if !assessment.allowed(force) {
                    return Err(format!(
                        "Removing this checkout deletes {}; confirm with force",
                        risks.join(", ")
                    )
                    .into());
                }
                let result = git.remove_space(session, &assessment, close, force).await?;
                let git = result.git.as_ref().map(|result| match result {
                    Ok(output) => git_value(output),
                    Err(error) => json!({"error":error.to_string()}),
                });
                return Ok(
                    json!({"ok":matches!(result.workspace.layout,WriteOutcome::Written)&&result.git.as_ref().is_some_and(|r|r.as_ref().is_ok_and(|o|o.code==Some(0))),"layout":layout_value(&result.workspace.layout),"observed":result.workspace.observed,"git":git}),
                );
            }
            _ => return Err("Unknown worktree command".into()),
        }
    }
    if let Some(placement) = placement {
        let folder = session
            .launch_directory(machine, &placement, text(&args, "folder"))
            .await?;
        let argv = match args.get("argv") {
            None => vec![],
            Some(value) => serde_json::from_value::<Vec<String>>(value.clone())?,
        };
        let result = session
            .create(
                machine,
                placement,
                Launch {
                    argv,
                    line: text(&args, "line").map(str::to_owned),
                    cwd: folder,
                    env: Default::default(),
                },
                id("op"),
            )
            .await?;
        if let Ok(doc) = session.document(machine) {
            focus_created(view, &doc, &pane);
        }
        let mut value = creation_value(&result);
        value["pane"] = json!(view.focused);
        value["tab"] = json!(view.active_tab);
        value["space"] = json!(view.active_space);
        return Ok(value);
    }
    Err(format!("Unknown command: {command}").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_client_lib::transport::HcConfig;
    #[test]
    fn launch_source_uses_explicit_placement() {
        use herdr_client_lib::document::{Document, terminal_pane};
        let mut doc = Document::default();
        doc.create_space("s", "", "/repo").unwrap();
        doc.create_tab("s", "t", "", terminal_pane("a", "term"))
            .unwrap();
        let split = Placement::Split {
            space: "s".into(),
            tab: "t".into(),
            target: "a".into(),
            container: "c".into(),
            pane: "b".into(),
            axis: Axis::Row,
        };
        assert_eq!(
            launch_source(&doc, &split),
            (Some("s".into()), Some("term".into()))
        );
        let tab = Placement::Tab {
            space: "s".into(),
            after: None,
            id: "t2".into(),
            name: "".into(),
            pane: "b".into(),
        };
        assert_eq!(launch_source(&doc, &tab), (Some("s".into()), None));
    }
    #[test]
    fn space_blocks_move_together_before_a_target() {
        use herdr_client_lib::document::Document;
        let mut doc = Document::default();
        for space in ["a", "b", "c", "d"] {
            doc.create_space(space, "", "").unwrap();
        }
        let order = |doc: &Document| {
            doc.value()["spaces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        move_spaces(&mut doc, &["d".into(), "b".into()], Some("a")).unwrap();
        assert_eq!(order(&doc), ["d", "b", "a", "c"]);
        move_spaces(&mut doc, &["d".into()], None).unwrap();
        assert_eq!(order(&doc), ["b", "a", "c", "d"]);
        assert!(move_spaces(&mut doc, &["b".into()], Some("b")).is_err());
    }
    #[test]
    fn creation_focus_leaves_zoom() {
        use herdr_client_lib::document::{Document, terminal_pane};
        let mut doc = Document::default();
        doc.create_space("s", "", "").unwrap();
        doc.create_tab("s", "t", "", terminal_pane("a", "1"))
            .unwrap();
        doc.split("a", "split", Axis::Row, terminal_pane("b", "2"))
            .unwrap();
        let mut view = View::default();
        view.focus(&doc, "a");
        view.zoomed = Some("a".into());
        focus_created(&mut view, &doc, "b");
        assert_eq!(view.focused.as_deref(), Some("b"));
        assert_eq!(view.zoomed, None);
        assert_eq!(
            crate::layout::panes(
                &doc.node("t").unwrap()["root"],
                ratatui::layout::Rect::new(0, 0, 80, 24),
                view.focused.as_deref(),
                view.zoomed.as_deref()
            )
            .len(),
            2
        );
    }
    #[tokio::test]
    #[ignore = "requires isolated HC_TUI_TEST_ROOT and core socket access"]
    async fn workspace_and_worktree_workflows() {
        let root = std::env::var("HC_TUI_TEST_ROOT").expect("scratch root");
        assert!(root.starts_with("/tmp/"));
        std::fs::create_dir_all(format!("{root}/runtime")).unwrap();
        let config = HcConfig {
            binary: "hc".into(),
            session: "tui-workflows".into(),
            environment: std::collections::BTreeMap::from([
                ("XDG_RUNTIME_DIR".into(), format!("{root}/runtime")),
                ("XDG_STATE_HOME".into(), format!("{root}/state")),
                ("XDG_CONFIG_HOME".into(), format!("{root}/config")),
            ]),
        };
        let timeout = std::time::Duration::from_secs(10);
        let mut session = Session::connect(
            config.connect(timeout).await.unwrap(),
            [String::new()],
            timeout,
        )
        .await
        .unwrap();
        let mut view = View::default();
        execute(
            &mut session,
            "",
            &mut view,
            "space.new",
            json!({"name":"Workflow","folder":root,"argv":["sh","-c","sleep 120"]}),
        )
        .await
        .unwrap();
        let space = view.active_space.clone().unwrap();
        let first = view.focused.clone().unwrap();
        execute(
            &mut session,
            "",
            &mut view,
            "pane.split",
            json!({"argv":["sh","-c","sleep 120"]}),
        )
        .await
        .unwrap();
        let second = view.focused.clone().unwrap();
        execute(
            &mut session,
            "",
            &mut view,
            "pane.resize",
            json!({"weight":2}),
        )
        .await
        .unwrap();
        execute(
            &mut session,
            "",
            &mut view,
            "pane.swap",
            json!({"other":first}),
        )
        .await
        .unwrap();
        assert!(session.document("").unwrap().contains(&second));
        let review = execute(&mut session, "", &mut view, "pane.close", json!({}))
            .await
            .unwrap();
        execute(
            &mut session,
            "",
            &mut view,
            "space.rename",
            json!({"name":"Changed"}),
        )
        .await
        .unwrap();
        assert!(
            execute(
                &mut session,
                "",
                &mut view,
                "pane.close",
                json!({"confirm":true,"expected":review})
            )
            .await
            .is_err()
        );
        let git = Git {
            client: session.client.clone(),
            machine: "".into(),
            deadline: timeout,
        };
        let repo = format!("{root}/repo");
        let path = format!("{root}/checkout");
        assert_eq!(
            git.exec(vec!["git".into(), "init".into(), repo.clone()], None)
                .await
                .unwrap()
                .code,
            Some(0)
        );
        assert_eq!(
            git.run(
                &repo,
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "initial"
                ]
            )
            .await
            .unwrap()
            .code,
            Some(0)
        );
        execute(
            &mut session,
            "",
            &mut view,
            "worktree.new",
            json!({"repository":repo,"path":path,"branch":"test"}),
        )
        .await
        .unwrap();
        assert_eq!(git.list(&repo).await.unwrap().len(), 2);
        execute(
            &mut session,
            "",
            &mut view,
            "worktree.remove",
            json!({"repository":repo,"path":path,"confirm":true}),
        )
        .await
        .unwrap();
        assert_eq!(git.list(&repo).await.unwrap().len(), 1);
        execute(
            &mut session,
            "",
            &mut view,
            "space.close",
            json!({"id":space,"confirm":true}),
        )
        .await
        .unwrap();
    }
}
