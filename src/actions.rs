use crate::Result;
use herdr_client_lib::{
    commands::{Launch, Placement},
    document::Axis,
    sync::{Session, WriteOutcome},
    view::View,
    worktrees::{CreatePlan, Git},
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
fn git_value(output: &herdr_client_lib::worktrees::Output) -> Value {
    json!({"code":output.code,"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
}
pub const HELP: &str = "space.new / tab.new / pane.split • name, folder, argv\nspace.rename / tab.rename / pane.rename • id, name\nspace.move / tab.move • id, index\npane.move • id, destination, index\npane.swap • id, other    pane.resize • id, weight\nspace.close / tab.close / pane.close • id, confirm\nterminal.keep • terminal    terminal.park • id\nworktree.list / worktree.branches • repository\nworktree.new • repository, branch, path, new_branch\nworktree.open • path    worktree.remove • repository, path, id, confirm\nMachine, focus, zoom, copy and search are private viewer actions.";

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
    let kind = command.split('.').next().unwrap_or("");
    if matches!(command, "space.rename" | "tab.rename" | "pane.rename") {
        let id = target(&args, view, kind)?;
        let name = required(&args, "name")?;
        return written(session.edit(machine, |d| d.rename(&id, name)).await?);
    }
    if matches!(command, "space.move" | "tab.move") {
        let id = target(&args, view, kind)?;
        let index = args["index"].as_u64().ok_or("index is required")? as usize;
        return written(session.edit(machine, |d| d.reorder(&id, index)).await?);
    }
    if matches!(command, "space.close" | "tab.close" | "pane.close") {
        let id = target(&args, view, kind)?;
        let assessment = session.assess_close(machine, &id).await?;
        let review = json!({"confirmation":"Closing ends these terminals and their dependent children", "target":id,"version":assessment.version,"terminals":assessment.terminals.iter().map(|t|json!({"address":t["address"],"pid":t["pid"],"parent":t["parent"],"labels":t["labels"]})).collect::<Vec<_>>(),"protected":assessment.protected});
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
    let folder = text(&args, "folder").map(str::to_owned).or_else(|| {
        view.active_space
            .as_deref()
            .and_then(|s| document.node(s))
            .and_then(|s| s["folder"].as_str())
            .map(str::to_owned)
    });
    let pane = id("p");
    let placement = match command {
        "space.new" | "worktree.new" | "worktree.open" => Some(Placement::Space {
            id: id("sp"),
            name: text(&args, "name")
                .or_else(|| text(&args, "branch"))
                .unwrap_or("Workspace")
                .into(),
            tab: id("tb"),
            pane: pane.clone(),
        }),
        "tab.new" | "terminal.keep" => Some(Placement::Tab {
            space: text(&args, "space")
                .map(str::to_owned)
                .or_else(|| view.active_space.clone())
                .ok_or("Select a space first")?,
            id: id("tb"),
            name: text(&args, "name").unwrap_or("Terminal").into(),
            pane: pane.clone(),
        }),
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
            view.focus(&doc, &pane);
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
                    view.focus(&doc, &pane);
                }
                return Ok(creation_value(&result));
            }
            "worktree.new" => {
                let repository = required(&args, "repository")?;
                let branch = required(&args, "branch")?;
                let path = match text(&args, "path") {
                    Some(path) => path.to_owned(),
                    None => git.default_path(repository, branch).await?,
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
                    view.focus(&doc, &pane);
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
                if !assessment.safe() {
                    return Err(format!("Checkout cannot be removed safely: {assessment:?}").into());
                }
                let close = session
                    .assess_close(machine, &target(&args, view, "space")?)
                    .await?;
                let review = json!({"confirmation":"Remove this checkout and close its workspace","path":assessment.checkout.path,"version":close.version,"assessment":format!("{assessment:?}"),"terminals":close.terminals.iter().map(|t|json!({"address":t["address"],"pid":t["pid"]})).collect::<Vec<_>>()});
                if args["confirm"] != true {
                    return Ok(review);
                }
                if args
                    .get("expected")
                    .is_some_and(|expected| expected != &review)
                {
                    return Err("Removal conditions changed. Review the checkout again.".into());
                }
                let result = git.remove_space(session, &assessment, close).await?;
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
            view.focus(&doc, &pane);
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
