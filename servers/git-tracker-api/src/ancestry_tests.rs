// SPDX-License-Identifier: FSL-1.1-ALv2
use crate::github::GitHub;
use axum::{
    Json, Router,
    extract::{Request, State as AxumState},
    http::StatusCode,
};
use delino_protocol::gittracker::v1::{Context, Repository};
use jsonwebtoken::EncodingKey;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const KEY: &[u8] = include_bytes!("../tests/fixtures/github-test-key.pem");

fn sha(index: usize) -> String {
    format!("{index:040x}")
}

#[derive(Clone)]
struct MockState {
    commits: Arc<Vec<Value>>,
    listed: Arc<Vec<Value>>,
    requested: Arc<Mutex<Vec<String>>>,
    delay: Duration,
}

struct Fixture {
    github: GitHub,
    repository: Repository,
    context: Context,
    requested: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(depth: usize, missing: Option<usize>, merge_sides: bool, delay: Duration) -> Self {
        let commits: Vec<Value> = (0..=depth)
            .map(|i| {
                let mut parents = Vec::new();
                if i > 0 {
                    parents.push(json!({"sha":sha(i - 1)}));
                }
                if merge_sides && i > 0 && i % 10 == 0 {
                    parents.push(json!({"sha":format!("f{i:039x}")}));
                }
                json!({"sha":sha(i),"parents":parents})
            })
            .collect();
        let mut listed = Vec::new();
        for i in (0..=depth).rev() {
            if missing != Some(i) {
                listed.push(commits[i].clone());
            }
            if merge_sides && i > 0 && i % 10 == 0 {
                listed.push(json!({"sha":format!("f{i:039x}"),"parents":[]}));
            }
        }
        let requested = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            commits: Arc::new(commits),
            listed: Arc::new(listed),
            requested: requested.clone(),
            delay,
        };
        let mock = Router::new()
            .fallback(|AxumState(state): AxumState<MockState>, request: Request| async move {
                let path = request.uri().path();
                let query = request.uri().query().unwrap_or("");
                state.requested.lock().unwrap().push(format!("{path}?{query}"));
                let response = if path == "/app/installations/1/access_tokens" {
                    json!({"token":"installation"})
                } else if path == "/repos/owner/repo/pulls/1" {
                    json!({"base":{"ref":"main","sha":sha(0)},"head":{"sha":state.commits.last().unwrap()["sha"]}})
                } else if path == "/repos/owner/repo/git/ref/heads/main" {
                    json!({"ref":"refs/heads/main","object":{"type":"commit","sha":state.commits.last().unwrap()["sha"]}})
                } else if let Some(commit) = path.strip_prefix("/repos/owner/repo/commits/") {
                    match state.commits.iter().find(|row| row["sha"] == commit) {
                        Some(row) => row.clone(),
                        None => return (StatusCode::NOT_FOUND, Json(json!({"message":"missing"}))),
                    }
                } else if path == "/repos/owner/repo/commits" {
                    tokio::time::sleep(state.delay).await;
                    let mut page = None;
                    let mut per_page = None;
                    let mut head = None;
                    for part in query.split('&') {
                        if let Some(value) = part.strip_prefix("page=") {
                            page = value.parse::<usize>().ok();
                        } else if let Some(value) = part.strip_prefix("per_page=") {
                            per_page = value.parse::<usize>().ok();
                        } else if let Some(value) = part.strip_prefix("sha=") {
                            head = Some(value);
                        }
                    }
                    if per_page != Some(100) || head != state.commits.last().unwrap()["sha"].as_str() {
                        return (StatusCode::BAD_REQUEST, Json(json!({"message":"invalid query"})));
                    }
                    let Some(page) = page.filter(|page| *page > 0) else {
                        return (StatusCode::BAD_REQUEST, Json(json!({"message":"invalid page"})));
                    };
                    json!(state.listed.iter().skip((page - 1) * 100).take(100).collect::<Vec<_>>())
                } else {
                    return (StatusCode::NOT_FOUND, Json(json!({"message":"missing"})));
                };
                (StatusCode::OK, Json(response))
            })
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            github: GitHub::new(
                http,
                base,
                "1".into(),
                EncodingKey::from_rsa_pem(KEY).unwrap(),
            ),
            repository: Repository {
                full_name: "owner/repo".into(),
                installation_id: "1".into(),
                ..Default::default()
            },
            context: Context {
                commit: sha(depth),
                branch: "main".into(),
                ..Default::default()
            },
            requested,
            task,
        }
    }

    fn calls(&self, path: &str) -> usize {
        self.requested
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with(path))
            .count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn linear_five_thousand_collects_one_thousand_with_bounded_requests() {
    let fixture = Fixture::new(5000, None, false, Duration::ZERO).await;
    let (ancestors, baseline) = fixture
        .github
        .resolve(&fixture.repository, &fixture.context)
        .await
        .unwrap();
    assert!(baseline.is_empty());
    assert_eq!(ancestors, (4000..5000).rev().map(sha).collect::<Vec<_>>());
    assert_eq!(fixture.calls("/repos/owner/repo/commits?"), 10);
    // The tenth list page contains the head and only 999 ancestors. One singleton fills the cap.
    assert_eq!(
        fixture.calls(&format!("/repos/owner/repo/commits/{}", sha(4000))),
        1
    );
    assert!(fixture.requested.lock().unwrap().len() < 30);
}

#[tokio::test]
async fn missing_first_parent_in_merge_pages_uses_one_fallback_and_keeps_order() {
    let fixture = Fixture::new(120, Some(115), true, Duration::ZERO).await;
    let (ancestors, _) = fixture
        .github
        .resolve(&fixture.repository, &fixture.context)
        .await
        .unwrap();
    assert_eq!(ancestors, (0..120).rev().map(sha).collect::<Vec<_>>());
    assert_eq!(
        fixture.calls(&format!("/repos/owner/repo/commits/{}", sha(115))),
        1
    );
    assert_eq!(fixture.calls("/repos/owner/repo/commits?"), 2);
}

#[tokio::test]
async fn ancestry_deadline_returns_github_unavailable() {
    let mut fixture = Fixture::new(120, None, false, Duration::from_millis(100)).await;
    fixture.github.ancestry_deadline = Duration::from_millis(10);
    let fault = fixture
        .github
        .resolve(&fixture.repository, &fixture.context)
        .await
        .unwrap_err();
    assert_eq!(fault.code, connectrpc::ErrorCode::Unavailable);
    assert_eq!(fault.source, server_common::error::FaultSource::GitHub);
}

#[tokio::test]
async fn root_commit_needs_no_list_request() {
    let fixture = Fixture::new(0, None, false, Duration::ZERO).await;
    let (ancestors, _) = fixture
        .github
        .resolve(&fixture.repository, &fixture.context)
        .await
        .unwrap();
    assert!(ancestors.is_empty());
    assert_eq!(fixture.calls("/repos/owner/repo/commits?"), 0);
}

#[tokio::test]
async fn explicit_push_baseline_and_pull_request_keep_their_existing_paths() {
    let mut fixture = Fixture::new(120, None, false, Duration::ZERO).await;
    fixture.context.baseline_commit = sha(20);
    let (ancestors, baseline) = fixture
        .github
        .resolve(&fixture.repository, &fixture.context)
        .await
        .unwrap();
    assert_eq!(ancestors.len(), 120);
    assert_eq!(baseline, sha(20));
    fixture.requested.lock().unwrap().clear();
    fixture.context.pull_request = 1;
    let (ancestors, baseline) = fixture
        .github
        .resolve(&fixture.repository, &fixture.context)
        .await
        .unwrap();
    assert!(ancestors.is_empty());
    assert_eq!(baseline, sha(20));
    assert_eq!(fixture.calls("/repos/owner/repo/commits?"), 0);
}
