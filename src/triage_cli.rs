use crate::cli::{self, TriageArgs, TriageCommands, TriageCreateArgs};
use crate::triage::http::{
    AttachRequest, CatalogItem, ExportResponse, QueueDetailResponse, RefreshRequest,
};
use crate::triage::import::{capture_path, discover, discover_from};
use crate::triage::models::{
    CreateQueue, DecisionPatch, PatchField, RefreshStats, SourceSpec, TriageQueue, TriageRevision,
    TriageVerdict,
};
use anyhow::{Context, Result};
use reqwest::{Client, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const CLOSED_QUEUE_EXIT_CODE: u8 = 3;

#[derive(Debug, thiserror::Error)]
pub enum TriageCliError {
    #[error("{message}")]
    ClosedQueue { message: String },
    #[error("{message}")]
    Server { status: StatusCode, message: String },
}

pub fn exit_code(error: &anyhow::Error) -> u8 {
    if matches!(
        error.downcast_ref::<TriageCliError>(),
        Some(TriageCliError::ClosedQueue { .. })
    ) {
        CLOSED_QUEUE_EXIT_CODE
    } else {
        1
    }
}

pub async fn run(args: TriageArgs) -> Result<()> {
    let TriageArgs { json, command } = args;
    let client = TriageClient::new(cli::daemon_url(None))?;

    match command {
        TriageCommands::Create(args) => {
            let request = create_request(args)?;
            let queue = client.create_queue(&request).await?;
            print_queue(&queue, json, "Created")?;
        }
        TriageCommands::List => {
            let queues = client.list_queues().await?;
            if json {
                print_json(&queues)?;
            } else if queues.is_empty() {
                println!("No triage queues.");
            } else {
                for queue in &queues {
                    print_queue_line(queue);
                }
            }
        }
        TriageCommands::Show { queue } => {
            let detail = client.get_queue(&queue).await?;
            if json {
                print_json(&detail)?;
            } else {
                print_queue_line(&detail.queue);
                for item in &detail.catalog {
                    let verdict = item
                        .latest_revision
                        .as_ref()
                        .and_then(|revision| revision.verdict)
                        .map_or("undecided", TriageVerdict::as_str);
                    let location = item.path.as_deref().unwrap_or("<inline>");
                    println!(
                        "  {}  {}  {}  {}",
                        item.id,
                        item.freshness.state.as_str(),
                        verdict,
                        location
                    );
                }
            }
        }
        TriageCommands::Decide {
            queue,
            item,
            verdict,
            target,
        } => {
            validate_decision_args(verdict, target.as_deref())?;
            let patch = DecisionPatch {
                verdict: PatchField::Value(verdict),
                target: target.map_or(PatchField::Missing, PatchField::Value),
            };
            let revision = client.decide(&queue, &item, &patch).await?;
            print_revision(&revision, json, false)?;
        }
        TriageCommands::Clear { queue, item } => {
            let patch = clear_patch();
            let revision = client.decide(&queue, &item, &patch).await?;
            print_revision(&revision, json, true)?;
        }
        TriageCommands::Refresh { queue } => {
            let queue = resolve_refresh_queue(&client, queue.as_deref()).await?;
            let stats = refresh_queue(&client, &queue).await?;
            if json {
                print_json(&stats)?;
            } else {
                print_refresh_stats(&stats);
            }
        }
        TriageCommands::Attach { queue, item, path } => {
            let detail = client.get_queue(&queue).await?;
            let queue_id = detail.queue.id.clone();
            let observation = capture_path(
                Path::new(&detail.queue.root),
                &path,
                &detail.queue.strip_suffix,
            )?;
            let request = AttachRequest {
                path: observation
                    .path
                    .context("captured attachment did not have a normalized path")?,
                markdown: observation.markdown,
            };
            let attached = client.attach(&queue_id, &item, &request).await?;
            if json {
                print_json(&attached)?;
            } else {
                println!(
                    "Attached {}/{} to {}",
                    queue_id,
                    attached.id,
                    attached.path.as_deref().unwrap_or("<inline>")
                );
            }
        }
        TriageCommands::Close { queue } => {
            let queue = client.set_status(&queue, "close").await?;
            print_queue(&queue, json, "Closed")?;
        }
        TriageCommands::Reopen { queue } => {
            let queue = client.set_status(&queue, "reopen").await?;
            print_queue(&queue, json, "Reopened")?;
        }
        TriageCommands::Export {
            queue,
            since,
            limit,
        } => {
            let export = client.export(&queue, since.as_deref(), limit).await?;
            print_json(&export)?;
        }
    }

    Ok(())
}

fn create_request(args: TriageCreateArgs) -> Result<CreateQueue> {
    let invocation_dir =
        std::env::current_dir().context("failed to resolve invocation directory")?;
    create_request_from(&invocation_dir, args)
}

fn create_request_from(invocation_dir: &Path, args: TriageCreateArgs) -> Result<CreateQueue> {
    let source_spec = if args.glob.is_empty() {
        let path = args
            .json_list
            .context("exactly one of --glob or --json-list is required")?;
        SourceSpec::JsonList {
            path: path_to_utf8(path)?,
        }
    } else {
        SourceSpec::Glob {
            patterns: args.glob,
        }
    };
    let discovered = discover_from(invocation_dir, &args.root, &args.strip_suffix, &source_spec)?;
    Ok(CreateQueue {
        name: args.name,
        root: discovered.root,
        strip_suffix: args.strip_suffix,
        source_spec: discovered.source_spec,
        items: discovered.items,
    })
}

fn path_to_utf8(path: PathBuf) -> Result<String> {
    path.into_os_string().into_string().map_err(|path| {
        anyhow::anyhow!("path is not valid UTF-8: {}", PathBuf::from(path).display())
    })
}

fn validate_decision_args(verdict: TriageVerdict, target: Option<&str>) -> Result<()> {
    match (verdict, target) {
        (TriageVerdict::MergeInto, None) => {
            anyhow::bail!("merge_into requires --target <item>")
        }
        (TriageVerdict::MergeInto, Some("")) => {
            anyhow::bail!("merge_into target must not be empty")
        }
        (TriageVerdict::MergeInto, Some(_)) | (_, None) => Ok(()),
        (_, Some(_)) => anyhow::bail!("--target is only valid for merge_into"),
    }
}

fn clear_patch() -> DecisionPatch {
    DecisionPatch {
        verdict: PatchField::Null,
        target: PatchField::Missing,
    }
}

async fn resolve_refresh_queue(client: &TriageClient, queue: Option<&str>) -> Result<String> {
    if let Some(queue) = queue {
        return Ok(queue.to_string());
    }

    let queues = client.list_queues().await?;
    if queues.len() == 1 {
        return Ok(queues[0].id.clone());
    }

    let invocation_dir = std::fs::canonicalize(std::env::current_dir()?)?;
    let matching = queues
        .iter()
        .filter(|queue| Path::new(&queue.root) == invocation_dir)
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [queue] => Ok(queue.id.clone()),
        [] if queues.is_empty() => anyhow::bail!("no triage queues exist; create one first"),
        [] => anyhow::bail!(
            "more than one triage queue exists; pass the queue id or name to `sjbis triage refresh`"
        ),
        _ => anyhow::bail!(
            "more than one triage queue uses {}; pass the queue id or name explicitly",
            invocation_dir.display()
        ),
    }
}

async fn refresh_queue(client: &TriageClient, queue: &str) -> Result<RefreshStats> {
    let detail = client.get_queue(queue).await?;

    // Discovery must finish before the POST. If the producer filesystem is
    // unavailable, the daemon never sees an empty observation set.
    let discovered = discover(
        Path::new(&detail.queue.root),
        &detail.queue.strip_suffix,
        &detail.queue.source_spec,
    )?;
    client.refresh(&detail.queue.id, discovered.items).await
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_queue(queue: &TriageQueue, json: bool, action: &str) -> Result<()> {
    if json {
        print_json(queue)
    } else {
        println!("{action} {} ({})", queue.id, queue.name);
        print_queue_line(queue);
        Ok(())
    }
}

fn print_queue_line(queue: &TriageQueue) {
    println!(
        "{}  {}  {}  {}/{} decided  {} stale",
        queue.id,
        queue.status.as_str(),
        queue.name,
        queue.counts.decided,
        queue.counts.total,
        queue.counts.stale
    );
}

fn print_revision(revision: &TriageRevision, json: bool, cleared: bool) -> Result<()> {
    if json {
        print_json(revision)
    } else {
        let verdict = revision
            .verdict
            .map_or("cleared".to_string(), |verdict| verdict.to_string());
        let target = revision
            .target
            .as_deref()
            .map_or(String::new(), |target| format!(" -> {target}"));
        let action = if cleared { "Cleared" } else { "Recorded" };
        println!(
            "{action} {}/{} revision {}: {verdict}{target}",
            revision.queue_id, revision.item_id, revision.revision
        );
        Ok(())
    }
}

fn print_refresh_stats(stats: &RefreshStats) {
    println!(
        "Refreshed: {} current, {} added, {} moved, {} missing, {} changed, {} ambiguous, {} inline replaced",
        stats.current,
        stats.added,
        stats.moved,
        stats.missing,
        stats.content_changed,
        stats.ambiguous,
        stats.inline_replaced
    );
}

struct TriageClient {
    base: Url,
    http: Client,
}

impl TriageClient {
    fn new(base: String) -> Result<Self> {
        let base = Url::parse(&base).context("invalid SJBIS daemon URL")?;
        Ok(Self {
            base,
            http: Client::new(),
        })
    }

    async fn create_queue(&self, request: &CreateQueue) -> Result<TriageQueue> {
        let url = self.endpoint(&["triage", "queues"])?;
        self.send_json(self.http.post(url).json(request)).await
    }

    async fn list_queues(&self) -> Result<Vec<TriageQueue>> {
        let url = self.endpoint(&["triage", "queues"])?;
        self.send_json(self.http.get(url)).await
    }

    async fn get_queue(&self, queue: &str) -> Result<QueueDetailResponse> {
        let url = self.endpoint(&["triage", "queues", queue])?;
        self.send_json(self.http.get(url)).await
    }

    async fn decide(
        &self,
        queue: &str,
        item: &str,
        patch: &DecisionPatch,
    ) -> Result<TriageRevision> {
        let url = self.endpoint(&["triage", "queues", queue, "items", item, "decision"])?;
        self.send_json(self.http.patch(url).json(patch)).await
    }

    async fn refresh(
        &self,
        queue: &str,
        items: Vec<crate::triage::models::Observation>,
    ) -> Result<RefreshStats> {
        let url = self.endpoint(&["triage", "queues", queue, "refresh"])?;
        self.send_json(self.http.post(url).json(&RefreshRequest { items }))
            .await
    }

    async fn attach(
        &self,
        queue: &str,
        item: &str,
        request: &AttachRequest,
    ) -> Result<CatalogItem> {
        let url = self.endpoint(&["triage", "queues", queue, "items", item, "attach"])?;
        self.send_json(self.http.post(url).json(request)).await
    }

    async fn set_status(&self, queue: &str, action: &str) -> Result<TriageQueue> {
        let url = self.endpoint(&["triage", "queues", queue, action])?;
        self.send_json(self.http.post(url)).await
    }

    async fn export(
        &self,
        queue: &str,
        since: Option<&str>,
        limit: Option<usize>,
    ) -> Result<ExportResponse> {
        let mut url = self.endpoint(&["triage", "queues", queue, "export"])?;
        if since.is_some() || limit.is_some() {
            let mut query = url.query_pairs_mut();
            if let Some(since) = since {
                query.append_pair("since", since);
            }
            if let Some(limit) = limit {
                query.append_pair("limit", &limit.to_string());
            }
        }
        self.send_json(self.http.get(url)).await
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.set_query(None);
        url.set_fragment(None);
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("SJBIS daemon URL cannot be used as an HTTP base"))?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
        drop(path);
        Ok(url)
    }

    async fn send_json<T: DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> Result<T> {
        let response = request
            .send()
            .await
            .context("failed to connect to SJBIS daemon")?;
        decode_response(response).await
    }
}

async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .context("failed to read SJBIS daemon response")?;
    if status.is_success() {
        return serde_json::from_slice(&bytes).context("failed to parse SJBIS daemon response");
    }

    let value = serde_json::from_slice::<Value>(&bytes).ok();
    let message = value
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| String::from_utf8(bytes.to_vec()).ok())
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| format!("SJBIS daemon returned HTTP {status}"));
    let code = value
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str);

    if status == StatusCode::CONFLICT && code == Some("queue_closed") {
        Err(TriageCliError::ClosedQueue { message }.into())
    } else {
        Err(TriageCliError::Server { status, message }.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::OriginalUri, routing::get};
    use serde_json::json;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::net::TcpListener;

    fn queue_json(root: &Path) -> Value {
        json!({
            "id": "tq-fixture",
            "name": "fixture",
            "root": root,
            "strip_suffix": "_analysis.md",
            "source_spec": {"kind": "glob", "patterns": ["**/*.md"]},
            "status": "open",
            "complete": false,
            "created_at": "2026-09-27T00:00:00Z",
            "updated_at": "2026-09-27T00:00:00Z",
            "closed_at": null,
            "counts": {"total": 0, "decided": 0, "stale": 0, "ambiguous": 0}
        })
    }

    #[test]
    fn decision_target_validation_matches_wire_contract() {
        for verdict in [
            TriageVerdict::Schedule,
            TriageVerdict::Delete,
            TriageVerdict::NeedsReplan,
            TriageVerdict::LeaveCaptured,
        ] {
            assert!(validate_decision_args(verdict, None).is_ok());
            assert!(validate_decision_args(verdict, Some("other")).is_err());
        }
        assert!(validate_decision_args(TriageVerdict::MergeInto, Some("other")).is_ok());
        assert!(validate_decision_args(TriageVerdict::MergeInto, None).is_err());
    }

    #[test]
    fn clear_serializes_an_explicit_null_verdict() {
        assert_eq!(
            serde_json::to_value(clear_patch()).unwrap(),
            json!({"verdict": null})
        );
    }

    #[test]
    fn create_resolves_relative_root_at_invocation_and_captures_normalized_items() {
        let invocation = tempfile::tempdir().unwrap();
        let root = invocation.path().join("notes");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("z_analysis.md"), "# Z\n").unwrap();
        std::fs::write(root.join("nested/a_analysis.md"), "# A\n").unwrap();
        let request = create_request_from(
            invocation.path(),
            TriageCreateArgs {
                name: "fixture".to_string(),
                root: PathBuf::from("notes"),
                glob: vec!["**/*.md".to_string()],
                json_list: None,
                strip_suffix: "_analysis.md".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            request.root,
            std::fs::canonicalize(root).unwrap().display().to_string()
        );
        assert_eq!(
            request
                .items
                .iter()
                .map(|item| item.path.as_deref())
                .collect::<Vec<_>>(),
            [Some("nested/a_analysis.md"), Some("z_analysis.md")]
        );
        assert_eq!(
            request
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        assert_eq!(request.items[0].markdown, "# A\n");
    }

    #[tokio::test]
    async fn closed_queue_conflict_has_exit_code_three() {
        let app = Router::new().route(
            "/triage/queues/q/items/i/decision",
            get(|| async {
                (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": "queue is closed; reopen first",
                        "code": "queue_closed"
                    })),
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = TriageClient::new(format!("http://{address}")).unwrap();
        let url = client
            .endpoint(&["triage", "queues", "q", "items", "i", "decision"])
            .unwrap();
        let error = client
            .send_json::<Value>(client.http.get(url))
            .await
            .unwrap_err();

        assert_eq!(exit_code(&error), CLOSED_QUEUE_EXIT_CODE);
        assert_eq!(error.to_string(), "queue is closed; reopen first");
    }

    #[tokio::test]
    async fn validation_errors_keep_the_server_message() {
        let app = Router::new().route(
            "/triage/queues/q/items/i/decision",
            get(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "merge_into target must name an existing queue item"})),
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = TriageClient::new(format!("http://{address}")).unwrap();
        let url = client
            .endpoint(&["triage", "queues", "q", "items", "i", "decision"])
            .unwrap();
        let error = client
            .send_json::<Value>(client.http.get(url))
            .await
            .unwrap_err();

        assert_eq!(exit_code(&error), 1);
        assert_eq!(
            error.to_string(),
            "merge_into target must name an existing queue item"
        );
    }

    #[tokio::test]
    async fn refresh_does_not_post_when_the_producer_filesystem_is_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        let missing_root = temp.path().join("missing-root");
        let detail = json!({"queue": queue_json(&missing_root), "catalog": []});
        let posts = Arc::new(AtomicUsize::new(0));
        let post_counter = Arc::clone(&posts);
        let app = Router::new()
            .route(
                "/triage/queues/q",
                get(move || {
                    let detail = detail.clone();
                    async move { Json(detail) }
                }),
            )
            .route(
                "/triage/queues/tq-fixture/refresh",
                axum::routing::post(move || {
                    let post_counter = Arc::clone(&post_counter);
                    async move {
                        post_counter.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "current": 0,
                            "missing": 0,
                            "content_changed": 0,
                            "ambiguous": 0,
                            "added": 0,
                            "moved": 0,
                            "inline_replaced": 0
                        }))
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = TriageClient::new(format!("http://{address}")).unwrap();

        let error = refresh_queue(&client, "q").await.unwrap_err();

        assert!(error.to_string().contains("missing-root"));
        assert_eq!(posts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn export_omits_or_preserves_an_empty_since_query() {
        let queries = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
        let seen_queries = Arc::clone(&queries);
        let app = Router::new().route(
            "/triage/queues/q/export",
            get(move |OriginalUri(uri): OriginalUri| {
                let seen_queries = Arc::clone(&seen_queries);
                async move {
                    seen_queries
                        .lock()
                        .unwrap()
                        .push(uri.query().map(str::to_string));
                    Json(json!({
                        "queue": queue_json(Path::new("/tmp/fixture")),
                        "catalog": [],
                        "items": [],
                        "nextCursor": "triage-v1-0"
                    }))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = TriageClient::new(format!("http://{address}")).unwrap();

        client.export("q", None, None).await.unwrap();
        client.export("q", Some(""), None).await.unwrap();

        assert_eq!(
            *queries.lock().unwrap(),
            vec![None, Some("since=".to_string())]
        );
    }
}
