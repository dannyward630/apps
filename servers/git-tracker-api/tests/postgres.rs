// SPDX-License-Identifier: FSL-1.1-ALv2
use axum::{Json, Router, extract::Request, http::StatusCode, routing::post};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use git_tracker_api::{Server, auth::Actions, github::GitHub};
use hmac::{Hmac, Mac};
use jsonwebtoken::{Algorithm, EncodingKey, Header, jwk::JwkSet};
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};
use serde_json::{Value, json};
use server_common::{
    authority::Authority,
    telemetry::{Metrics, Service},
};
use sha2::Sha256;
use std::{sync::Arc, time::Duration};
const KEY: &[u8] = include_bytes!("fixtures/github-test-key.pem");
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_permissions_atomic_upload_oidc_visibility_and_cleanup() {
    permissions_atomic_upload_oidc_visibility_and_cleanup(chrono::Duration::days(90), false).await;
}
// A retention accepted at startup but too large for any representable cutoff must not stop the
// ordinary maintenance duties and must keep every audit row.
#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_full_cycle_with_unrepresentable_audit_retention() {
    let retention = chrono::Duration::from_std(Duration::from_secs(9_999_999_999 * 3600)).unwrap();
    permissions_atomic_upload_oidc_visibility_and_cleanup(retention, true).await;
}
async fn permissions_atomic_upload_oidc_visibility_and_cleanup(
    retention: chrono::Duration,
    retains_audit: bool,
) {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("tracker_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let search = schema.clone();
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .after_connect(move |c, _| {
            let search = search.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path TO {search}"))
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/00001_git_tracker.sql"))
        .execute(&db)
        .await
        .unwrap();
    let actor = uuid::Uuid::now_v7().to_string();
    let space = uuid::Uuid::now_v7().to_string();
    let commit = "a".repeat(40);
    let commit_mock = commit.clone();
    let actor_mock = actor.clone();
    let finalized = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let finalized_mock = finalized.clone();
    // Per-page delay (ms) for the optional PR comment listing; zero keeps the immediate 401.
    let page_delay = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let page_delay_mock = page_delay.clone();
    let pages_served = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let pages_served_mock = pages_served.clone();
    let mock=Router::new().route("/delcore.v1.DelcoreService/CheckTenantLifecycle",post(move||{let finalized=finalized_mock.clone();async move{Json(json!({"finalizedTenantIds":*finalized.lock().unwrap()}))}})).fallback(move|r:Request|{let commit=commit_mock.clone();let actor=actor_mock.clone();let page_delay=page_delay_mock.clone();let pages_served=pages_served_mock.clone();async move{
        let p=r.uri().path();
        let delay=page_delay.load(std::sync::atomic::Ordering::SeqCst);
        if delay>0&&p=="/repos/owner/repo/issues/1/comments"{
            tokio::time::sleep(Duration::from_millis(delay)).await;
            pages_served.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            // A full page without a Git Tracker comment forces pagination to continue.
            return (StatusCode::OK,Json(json!((0..100).map(|i|json!({"id":i+1,"body":"unrelated"})).collect::<Vec<_>>())));
        }let auth=r.headers().get("authorization").and_then(|v|v.to_str().ok()).unwrap_or("");
        let out=match p{
            "/token"=>json!({"access_token":"service","expires_in":3600}),
            "/delcore.v1.DelcoreService/GetCurrentUser" if auth=="Bearer manager"=>json!({"userId":actor}),
            "/delcore.v1.DelcoreService/ListMembers" if auth=="Bearer manager"=>json!({}),
            "/delcore.v1.DelcoreService/Authorize"=>json!({"allowed":true}),
            "/user"=>json!({"id":7}),
            "/repos/owner/repo/pulls/1"=>json!({"base":{"ref":"main","sha":commit},"head":{"sha":commit}}),
            "/repos/owner/repo"|"/repositories/42"=>json!({"id":42,"full_name":"owner/repo","private":false,"permissions":{"admin":true}}),
            "/repos/owner/repo/installation"=>json!({"id":1}),
            "/app/installations/1/access_tokens"=>json!({"token":"installation"}),
            // Exact branch-head lookups. The generic commit route below also resolves SHAs and tags, so
            // only these routes may make a caller-supplied value a branch. Unlisted names (a tag such as
            // `v1`, or a commit SHA) answer 401 like a missing ref.
            "/repos/owner/repo/git/ref/heads/main"=>json!({"ref":"refs/heads/main","object":{"type":"commit","sha":commit}}),
            "/repos/owner/repo/git/ref/heads/release/1%2E0"=>json!({"ref":"refs/heads/release/1.0","object":{"type":"commit","sha":commit}}),
            p if p==format!("/repos/owner/repo/git/ref/heads/{}","b".repeat(40))=>json!({"ref":format!("refs/heads/{}","b".repeat(40)),"object":{"type":"commit","sha":commit}}),
            "/repos/owner/repo/git/ref/heads/mismatch"=>json!({"ref":"refs/heads/other","object":{"type":"commit","sha":commit}}),
            "/repos/owner/repo/git/ref/heads/annotated"=>json!({"ref":"refs/heads/annotated","object":{"type":"tag","sha":commit}}),
            "/repos/owner/repo/git/ref/heads/badsha"=>json!({"ref":"refs/heads/badsha","object":{"type":"commit","sha":"not-a-sha"}}),
            p if p.starts_with("/repos/owner/repo/commits/")=>json!({"sha":commit,"parents":[]}),
            _=>return (StatusCode::UNAUTHORIZED,Json(json!({"code":"unauthenticated"}))),
        };(StatusCode::OK,Json(out))
    }});
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mock_task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let private = RsaPrivateKey::from_pkcs8_pem(std::str::from_utf8(KEY).unwrap()).unwrap();
    let jwks:JwkSet=serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"fixture","alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(private.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(private.e().to_bytes_be())}]})).unwrap();
    let server = Arc::new(Server {
        db: db.clone(),
        authority: Authority::new(
            http.clone(),
            base.clone(),
            format!("{base}/token"),
            "resource".into(),
            "client".into(),
            "fixture".into(),
        ),
        github: GitHub::new(
            http.clone(),
            base.clone(),
            "1".into(),
            EncodingKey::from_rsa_pem(KEY).unwrap(),
        ),
        actions: Actions::new(http.clone(), format!("{base}/jwks"), jwks),
        audit_retention: retention,
        webhook_secret: "fixture-only".into(),
        metrics: Arc::new(Metrics::new(Service::GitTracker)),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/gittracker.v1.GitTrackerService",
        listener.local_addr().unwrap()
    );
    let router = Router::new()
        .fallback_service(
            connectrpc::Router::new()
                .add_service(server.clone())
                .into_axum_service(),
        )
        // Same ingress stamp as the production router; without it publication is skipped.
        .layer(axum::middleware::from_fn(git_tracker_api::capture_ingress));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    async fn call(
        http: &reqwest::Client,
        base: &str,
        method: &str,
        authorization: &str,
        q: Value,
    ) -> (StatusCode, Value) {
        let r = http
            .post(format!("{base}/{method}"))
            .header("Authorization", authorization)
            .header("Connect-Protocol-Version", "1")
            .json(&q)
            .send()
            .await
            .unwrap();
        let status = r.status();
        assert!(r.headers().contains_key("X-Correlation-Id"));
        (status, r.json().await.unwrap())
    }
    let (status, connected) = call(
        &http,
        &base,
        "ConnectRepository",
        "Bearer manager",
        json!({"spaceId":space,"fullName":"owner/repo","githubAccessToken":"fixture"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap().to_owned();
    let (_,issued)=call(&http,&base,"IssueToken","Bearer manager",json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()})).await;
    let auth = format!("Bearer {}", issued["token"].as_str().unwrap());
    let context = json!({"repositoryId":rid,"executionId":"run","commit":commit,"branch":"main"});
    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let upload = |id: &str| json!({"context":context,"dataset":dataset,"rows":[{"id":id,"values":{"bytes":"9007199254740993.00000000000001"}}]});
    let (a, b) = tokio::join!(
        call(&http, &base, "Upload", &auth, upload("one")),
        call(&http, &base, "Upload", &auth, upload("two"))
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    let (_, execution) = call(
        &http,
        &base,
        "GetExecution",
        "",
        json!({"repositoryId":rid,"executionId":"run"}),
    )
    .await;
    assert_eq!(
        execution["execution"]["measurements"][0]["rows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (status, complete) =
        call(&http, &base, "Complete", &auth, json!({"context":context})).await;
    assert_eq!(status, StatusCode::OK, "{complete}");
    let (_, again) = call(&http, &base, "Complete", &auth, json!({"context":context})).await;
    assert_eq!(complete, again);
    let (_, closed) = call(&http, &base, "Upload", &auth, upload("one")).await;
    assert_eq!(closed["code"], "failed_precondition");
    let (_, history) = call(
        &http,
        &base,
        "GetHistory",
        "",
        json!({"repositoryId":rid,"branch":"main"}),
    )
    .await;
    assert_eq!(history["executions"][0]["representative"], true);
    let (_, compare) = call(
        &http,
        &base,
        "Compare",
        "",
        json!({"repositoryId":rid,"baselineCommit":commit,"targetCommit":commit}),
    )
    .await;
    assert_eq!(
        compare["comparisons"][0]["status"],
        "COMPARISON_STATUS_UNCHANGED"
    );
    let now = chrono::Utc::now().timestamp();
    let claims = json!({"iss":"https://token.actions.githubusercontent.com","aud":"https://git-tracker.api.delino.io","exp":now+300,"iat":now,"sub":"repo:owner/repo:ref:refs/heads/main","repository_id":"42","run_id":"1","run_attempt":"1","sha":commit,"ref":"refs/heads/main","event_name":"push"});
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("fixture".into());
    let token =
        jsonwebtoken::encode(&header, &claims, &EncodingKey::from_rsa_pem(KEY).unwrap()).unwrap();
    let mut c = context.clone();
    c["executionId"] = "42/1/1".into();
    let (status, v) = call(
        &http,
        &base,
        "Complete",
        &format!("Actions {token}"),
        json!({"context":c}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    c["executionId"] = "42/other/1".into();
    let (_, denied) = call(
        &http,
        &base,
        "Complete",
        &format!("Actions {token}"),
        json!({"context":c}),
    )
    .await;
    assert_eq!(denied["code"], "permission_denied");
    let github_failures = server
        .metrics
        .events
        .with_label_values(&["github", "failure"]);
    let before = github_failures.get();
    let mut bad_commit = context.clone();
    bad_commit["executionId"] = json!("bad-commit");
    bad_commit["commit"] = json!("b".repeat(40));
    let (_, failed) = call(
        &http,
        &base,
        "Upload",
        &auth,
        json!({"context":bad_commit,"dataset":dataset,"rows":[]}),
    )
    .await;
    assert_eq!(failed["code"], "unavailable");
    assert_eq!(github_failures.get(), before + 1);
    let mut pr_context = context.clone();
    pr_context["executionId"] = json!("pr-warning");
    pr_context["pullRequest"] = json!("1");
    pr_context["baselineCommit"] = json!(commit);
    let (status, warning) = call(
        &http,
        &base,
        "Upload",
        &auth,
        json!({"context":pr_context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{warning}");
    assert_eq!(warning["warnings"][0], "PR comment could not be updated");
    assert_eq!(github_failures.get(), before + 2);
    // Slow optional publication must not turn a saved upload into a timeout: every listing page
    // succeeds within its own limit, but the pages together exceed the client deadline.
    page_delay.store(700, std::sync::atomic::Ordering::SeqCst);
    let mut slow_context = pr_context.clone();
    slow_context["executionId"] = json!("pr-slow");
    let started = std::time::Instant::now();
    let slow = http
        .post(format!("{base}/Upload"))
        .header("Authorization", &auth)
        .header("Connect-Protocol-Version", "1")
        .header("Connect-Timeout-Ms", "4000")
        .json(&json!({"context":slow_context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(slow.status(), StatusCode::OK);
    let slow: Value = slow.json().await.unwrap();
    assert_eq!(slow["warnings"][0], "PR comment could not be updated");
    // Cutoff is the client deadline minus the reserve, so the response arrives before 4 s.
    assert!(
        started.elapsed() < Duration::from_millis(3900),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(github_failures.get(), before + 3);
    assert!(pages_served.load(std::sync::atomic::Ordering::SeqCst) < 4);
    // The execution is committed and the repository lock was released by the timed-out publisher.
    let (status, saved) = call(
        &http,
        &base,
        "GetExecution",
        "",
        json!({"repositoryId":rid,"executionId":"pr-slow"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["execution"]["measurements"][0]["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    page_delay.store(0, std::sync::atomic::Ordering::SeqCst);
    let lock_probe = std::time::Instant::now();
    let (status, after) = call(
        &http,
        &base,
        "Complete",
        &auth,
        json!({"context":slow_context}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert!(lock_probe.elapsed() < Duration::from_secs(2));
    // A client deadline shorter than the reserve leaves no publication budget at all.
    let mut spent_context = pr_context.clone();
    spent_context["executionId"] = json!("pr-spent");
    let spent = http
        .post(format!("{base}/Upload"))
        .header("Authorization", &auth)
        .header("Connect-Protocol-Version", "1")
        .header("Connect-Timeout-Ms", "1500")
        .json(&json!({"context":spent_context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(spent.status(), StatusCode::OK);
    let spent: Value = spent.json().await.unwrap();
    assert_eq!(spent["warnings"][0], "PR comment could not be updated");
    assert_eq!(github_failures.get(), before + 5);
    // Only an exact `refs/heads/<branch>` head may name a branch. Commit SHAs, tags and mismatching or
    // non-commit refs fail with the GitHub unavailable fault and persist nothing.
    let recorded = |id: String| {
        let db = db.clone();
        let rid = rid.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT jsonb_exists(state->'records',$2) FROM git_tracker_repositories WHERE id::text=$1",
            )
            .bind(rid)
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let rejected = [
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "v1",
        "mismatch",
        "annotated",
        "badsha",
        "main/",
        "a//b",
        "../main",
    ];
    for (i, branch) in rejected.iter().enumerate() {
        let id = format!("rejected-upload-{i}");
        let mut c = context.clone();
        c["executionId"] = json!(&id);
        c["branch"] = json!(branch);
        let (_, failed) = call(
            &http,
            &base,
            "Upload",
            &auth,
            json!({"context":c,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}),
        )
        .await;
        assert_eq!(failed["code"], "unavailable", "upload branch {branch}");
        assert!(!recorded(id.clone()).await, "upload branch {branch}");
        let id = format!("rejected-complete-{i}");
        c["executionId"] = json!(&id);
        let (_, failed) = call(&http, &base, "Complete", &auth, json!({"context":c})).await;
        assert_eq!(failed["code"], "unavailable", "complete branch {branch}");
        assert!(!recorded(id.clone()).await, "complete branch {branch}");
        let (_, failed) = call(
            &http,
            &base,
            "GetHistory",
            "",
            json!({"repositoryId":rid,"branch":branch}),
        )
        .await;
        assert_eq!(failed["code"], "unavailable", "history branch {branch}");
    }
    for (i, branch) in ["release/1.0", &"b".repeat(40)].iter().enumerate() {
        let id = format!("exact-upload-{i}");
        let mut c = context.clone();
        c["executionId"] = json!(&id);
        c["branch"] = json!(branch);
        let (status, v) = call(
            &http,
            &base,
            "Upload",
            &auth,
            json!({"context":c,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "upload branch {branch}: {v}");
        assert!(recorded(id.clone()).await);
        let id = format!("exact-complete-{i}");
        c["executionId"] = json!(&id);
        let (status, v) = call(&http, &base, "Complete", &auth, json!({"context":c})).await;
        assert_eq!(status, StatusCode::OK, "complete branch {branch}: {v}");
        let (status, v) = call(
            &http,
            &base,
            "GetHistory",
            "",
            json!({"repositoryId":rid,"branch":branch}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "history branch {branch}: {v}");
    }
    let (_, revoked) = call(
        &http,
        &base,
        "RevokeToken",
        "Bearer manager",
        json!({"repositoryId":rid,"tokenId":issued["tokenId"]}),
    )
    .await;
    assert!(revoked.get("code").is_none());
    let (_, denied) = call(&http, &base, "Complete", &auth, json!({"context":context})).await;
    assert_eq!(denied["code"], "unauthenticated");
    let event = serde_json::to_vec(&json!({"action":"deleted","installation":{"id":1}})).unwrap();
    assert!(
        server
            .webhook("installation", "sha256=00", &event)
            .await
            .is_err()
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-only").unwrap();
    mac.update(&event);
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    server
        .webhook("installation", &signature, &event)
        .await
        .unwrap();
    let (_, private) = call(
        &http,
        &base,
        "GetExecution",
        "",
        json!({"repositoryId":rid,"executionId":"run"}),
    )
    .await;
    assert_eq!(private["code"], "unauthenticated");
    // Only committed transitions contribute to interruption alerts.
    let events = server
        .metrics
        .events
        .with_label_values(&["execution", "interrupted"]);
    let before = events.get();
    let state: Value =
        sqlx::query_scalar("SELECT state FROM git_tracker_repositories WHERE id::text=$1")
            .bind(&rid)
            .fetch_one(&db)
            .await
            .unwrap();
    let mut stale = state["records"]["run"].clone();
    stale["execution"]["state"] = json!("EXECUTION_STATE_MEASURING");
    stale["execution"]["context"]["executionId"] = json!("stale-rpc");
    stale["last_data"] = json!((chrono::Utc::now() - chrono::Duration::hours(7)).to_rfc3339());
    sqlx::query("UPDATE git_tracker_repositories SET state=jsonb_set(state,ARRAY['records','stale-rpc'],$2) WHERE id::text=$1")
        .bind(&rid).bind(&stale).execute(&db).await.unwrap();
    sqlx::query("ALTER TABLE git_tracker_repositories ADD CONSTRAINT reject_interruption CHECK ((state #>> '{records,stale-rpc,execution,state}') <> 'EXECUTION_STATE_INTERRUPTED')")
        .execute(&db).await.unwrap();
    let query = json!({"repositoryId":rid,"executionId":"run"});
    let (_, failed) = call(
        &http,
        &base,
        "GetExecution",
        "Bearer manager",
        query.clone(),
    )
    .await;
    assert_eq!(failed["code"], "unavailable");
    assert_eq!(events.get(), before);
    sqlx::query("ALTER TABLE git_tracker_repositories DROP CONSTRAINT reject_interruption")
        .execute(&db)
        .await
        .unwrap();
    let (status, _) = call(
        &http,
        &base,
        "GetExecution",
        "Bearer manager",
        query.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(events.get(), before + 1);
    call(&http, &base, "GetExecution", "Bearer manager", query).await;
    assert_eq!(events.get(), before + 1);
    stale["execution"]["context"]["executionId"] = json!("stale-maintenance");
    sqlx::query("UPDATE git_tracker_repositories SET state=jsonb_set(state,ARRAY['records','stale-maintenance'],$2) WHERE id::text=$1")
        .bind(&rid).bind(&stale).execute(&db).await.unwrap();
    // An audit row of an unrelated space that is far older than any ordinary retention.
    let old_audit = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO git_tracker_audit VALUES($1,$2,'old_fixture',$3,'2000-01-01T00:00:00Z')",
    )
    .bind(old_audit)
    .bind(uuid::Uuid::now_v7())
    .bind(uuid::Uuid::now_v7())
    .execute(&db)
    .await
    .unwrap();
    server.maintain().await.unwrap();
    assert_eq!(events.get(), before + 2);
    server.maintain().await.unwrap();
    assert_eq!(events.get(), before + 2);
    let old_audit_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_tracker_audit WHERE id=$1")
            .bind(old_audit)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(old_audit_rows, i64::from(retains_audit));
    let (status, _) = call(
        &http,
        &base,
        "Delete",
        "Bearer manager",
        json!({"repositoryId":rid,"kind":"DELETE_KIND_EXECUTION","id":"run","confirmed":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    finalized.lock().unwrap().push(space);
    server.maintain().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM git_tracker_repositories")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let old_audit_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_tracker_audit WHERE id=$1")
            .bind(old_audit)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(old_audit_rows, i64::from(retains_audit));
    let failures = server
        .metrics
        .events
        .with_label_values(&["persistence", "failure"]);
    let before = failures.get();
    sqlx::query("DROP TABLE git_tracker_repositories CASCADE")
        .execute(&db)
        .await
        .unwrap();
    let (_, failed) = call(&http, &base, "ListRepositories", "", json!({})).await;
    assert_eq!(failed["code"], "unavailable");
    assert_eq!(failures.get(), before + 1);
    task.abort();
    mock_task.abort();
    drop(server);
    db.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

/// GitHub visibility state and one-shot gate shared with the metadata mock.
///
/// The gate parks the next `/repositories/42` response so a test can hold a webhook or maintenance
/// transaction open (it holds the repository row lock while it fetches metadata) and order a
/// connection attempt behind it without sleeping.
struct GitHubMock {
    private: std::sync::atomic::AtomicBool,
    fail_metadata: std::sync::atomic::AtomicBool,
    user_status: std::sync::atomic::AtomicU16,
    repository_status: std::sync::atomic::AtomicU16,
    repository_admin: std::sync::atomic::AtomicBool,
    repository_delay_ms: std::sync::atomic::AtomicU64,
    gate: std::sync::Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
    resolve_gate: std::sync::Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
}

struct Fixture {
    server: Arc<Server>,
    http: reqwest::Client,
    base: String,
    github: Arc<GitHubMock>,
    delcore_requests: Arc<std::sync::atomic::AtomicU64>,
    schema: String,
    admin: sqlx::PgPool,
    space: String,
    commit: String,
    finalized: Arc<std::sync::Mutex<Vec<String>>>,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

async fn fixture() -> Fixture {
    fixture_with_github_timeout(Duration::from_secs(20)).await
}

async fn fixture_with_github_timeout(github_timeout: Duration) -> Fixture {
    use std::sync::atomic::Ordering;
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("tracker_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let search = schema.clone();
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .after_connect(move |c, _| {
            let search = search.clone();
            Box::pin(async move {
                // The application name lets a test find its own lock waiters in pg_stat_activity.
                sqlx::query(&format!("SET search_path TO {search}"))
                    .execute(&mut *c)
                    .await?;
                sqlx::query(&format!("SET application_name TO '{search}'"))
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/00001_git_tracker.sql"))
        .execute(&db)
        .await
        .unwrap();
    let actor = uuid::Uuid::now_v7().to_string();
    let space = uuid::Uuid::now_v7().to_string();
    let commit = "a".repeat(40);
    let github = Arc::new(GitHubMock {
        private: false.into(),
        fail_metadata: false.into(),
        user_status: 200.into(),
        repository_status: 200.into(),
        repository_admin: true.into(),
        repository_delay_ms: 0.into(),
        gate: std::sync::Mutex::new(None),
        resolve_gate: std::sync::Mutex::new(None),
    });
    let finalized = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let delcore_requests = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let delcore_mock = delcore_requests.clone();
    let (commit_mock, actor_mock, github_mock) = (commit.clone(), actor.clone(), github.clone());
    let finalized_mock = finalized.clone();
    let mock = Router::new().fallback(move |r: Request| {
        let (commit, actor, github) = (commit_mock.clone(), actor_mock.clone(), github_mock.clone());
        let finalized = finalized_mock.clone();
        let delcore_requests = delcore_mock.clone();
        async move {
            let p = r.uri().path().to_owned();
            if p.starts_with("/delcore.v1.DelcoreService/") {
                delcore_requests.fetch_add(1, Ordering::SeqCst);
            }
            let auth = r
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            if p == "/repositories/42" {
                if github.fail_metadata.load(Ordering::SeqCst) {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"code":"unauthenticated"})));
                }
                let gate = github.gate.lock().unwrap().take();
                if let Some((entered, release)) = gate {
                    entered.notify_one();
                    release.notified().await;
                }
            }
            if p.starts_with("/repos/owner/repo/commits/") {
                let gate = github.resolve_gate.lock().unwrap().take();
                if let Some((entered, release)) = gate {
                    entered.notify_one();
                    release.notified().await;
                }
            }
            if p == "/repos/owner/repo" {
                let delay = github.repository_delay_ms.load(Ordering::SeqCst);
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }
            let status = match p.as_str() {
                "/user" => github.user_status.load(Ordering::SeqCst),
                "/repos/owner/repo" => github.repository_status.load(Ordering::SeqCst),
                _ => 200,
            };
            if status != 200 {
                return (
                    StatusCode::from_u16(status).unwrap(),
                    Json(json!({"message":"provider-body-must-stay-private"})),
                );
            }
            let private = github.private.load(Ordering::SeqCst);
            let out = match p.as_str() {
                "/token" => json!({"access_token":"service","expires_in":3600}),
                "/delcore.v1.DelcoreService/GetCurrentUser" if auth == "Bearer manager" => {
                    json!({"userId":actor})
                }
                "/delcore.v1.DelcoreService/ListMembers" if auth == "Bearer manager" => json!({}),
                "/delcore.v1.DelcoreService/Authorize" => json!({"allowed":true}),
                "/delcore.v1.DelcoreService/CheckTenantLifecycle" => {
                    let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let requested = request["tenantIds"].as_array().unwrap();
                    let available = finalized.lock().unwrap();
                    json!({"finalizedTenantIds":available.iter().filter(|id| requested.iter().any(|value| value.as_str() == Some(id.as_str()))).collect::<Vec<_>>()})
                }
                "/user" => json!({"id":7}),
                // Discovery deliberately reports the repository as public so a stale snapshot
                // would be visible if the connection persisted it.
                "/repos/owner/repo" => {
                    json!({"id":42,"full_name":"owner/repo","private":false,"permissions":{"admin":github.repository_admin.load(Ordering::SeqCst)}})
                }
                "/repositories/42" => json!({"id":42,"full_name":"owner/repo","private":private}),
                "/repos/owner/repo/installation" => json!({"id":1}),
                "/app/installations/1/access_tokens" => json!({"token":"installation"}),
                "/repos/owner/repo/pulls/1" => {
                    json!({"base":{"ref":"main","sha":commit},"head":{"sha":commit}})
                }
                "/repos/owner/repo/git/ref/heads/main" => {
                    json!({"ref":"refs/heads/main","object":{"type":"commit","sha":commit}})
                }
                p if p.starts_with("/repos/owner/repo/commits/") => {
                    json!({"sha":commit,"parents":[]})
                }
                _ => return (StatusCode::UNAUTHORIZED, Json(json!({"code":"unauthenticated"}))),
            };
            (StatusCode::OK, Json(out))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mock_task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let github_http = reqwest::Client::builder()
        .timeout(github_timeout)
        .build()
        .unwrap();
    let private_key = RsaPrivateKey::from_pkcs8_pem(std::str::from_utf8(KEY).unwrap()).unwrap();
    let jwks: JwkSet = serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"fixture","alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(private_key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(private_key.e().to_bytes_be())}]})).unwrap();
    let server = Arc::new(Server {
        db,
        authority: Authority::new(
            http.clone(),
            base.clone(),
            format!("{base}/token"),
            "resource".into(),
            "client".into(),
            "fixture".into(),
        ),
        github: GitHub::new(
            github_http,
            base.clone(),
            "1".into(),
            EncodingKey::from_rsa_pem(KEY).unwrap(),
        ),
        actions: Actions::new(http.clone(), format!("{base}/jwks"), jwks),
        audit_retention: chrono::Duration::days(90),
        webhook_secret: "fixture-only".into(),
        metrics: Arc::new(Metrics::new(Service::GitTracker)),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/gittracker.v1.GitTrackerService",
        listener.local_addr().unwrap()
    );
    let router = Router::new()
        .fallback_service(
            connectrpc::Router::new()
                .add_service(server.clone())
                .into_axum_service(),
        )
        .layer(axum::middleware::from_fn(git_tracker_api::capture_ingress));
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Fixture {
        server,
        http,
        base,
        github,
        delcore_requests,
        schema,
        admin,
        space,
        commit,
        finalized,
        _tasks: vec![mock_task, task],
    }
}

impl Fixture {
    async fn call(&self, method: &str, authorization: &str, q: Value) -> (StatusCode, Value) {
        let r = self
            .http
            .post(format!("{}/{method}", self.base))
            .header("Authorization", authorization)
            .header("Connect-Protocol-Version", "1")
            .json(&q)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap())
    }
    async fn connect(&self) -> (StatusCode, Value) {
        self.connect_with("owner/repo", "fixture").await
    }
    async fn connect_with(&self, full_name: &str, access_token: &str) -> (StatusCode, Value) {
        self.call(
            "ConnectRepository",
            "Bearer manager",
            json!({"spaceId":self.space,"fullName":full_name,"githubAccessToken":access_token}),
        )
        .await
    }
    /// Arms the one-shot metadata gate and returns `(entered, release)`.
    fn arm_gate(&self) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
        let gate = (
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(tokio::sync::Notify::new()),
        );
        *self.github.gate.lock().unwrap() = Some(gate.clone());
        gate
    }
    fn arm_resolve_gate(&self) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
        let gate = (
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(tokio::sync::Notify::new()),
        );
        *self.github.resolve_gate.lock().unwrap() = Some(gate.clone());
        gate
    }
    /// Waits until a statement of this fixture's schema is blocked on a lock.
    async fn wait_for_lock_waiter(&self) {
        for _ in 0..500 {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name=$1 AND wait_event_type='Lock'",
            )
            .bind(&self.schema)
            .fetch_one(&self.admin)
            .await
            .unwrap();
            if waiting > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the connection never blocked on the repository lock");
    }
    async fn row(&self) -> Option<(String, bool, bool)> {
        sqlx::query(&format!(
            "SELECT id::text,public,connected FROM {}.git_tracker_repositories",
            self.schema
        ))
        .fetch_optional(&self.admin)
        .await
        .unwrap()
        .map(|r| {
            use sqlx::Row;
            (r.get(0), r.get(1), r.get(2))
        })
    }
    async fn audit_count(&self) -> i64 {
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {}.git_tracker_audit",
            self.schema
        ))
        .fetch_one(&self.admin)
        .await
        .unwrap()
    }
    async fn upload(&self, rid: &str) {
        let (_, issued) = self
            .call(
                "IssueToken",
                "Bearer manager",
                json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
            )
            .await;
        let auth = format!("Bearer {}", issued["token"].as_str().unwrap());
        let context =
            json!({"repositoryId":rid,"executionId":"run","commit":self.commit,"branch":"main"});
        let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
        let (status, body) = self
            .call(
                "Upload",
                &auth,
                json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    /// Asserts private visibility: anonymous reads are denied and listings omit the repository,
    /// while an authorized member still reads the retained history.
    async fn assert_private_with_history(&self, rid: &str) {
        let (status, listed) = self
            .call("ListRepositories", "", json!({"spaceId":""}))
            .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(
            listed["repositories"]
                .as_array()
                .is_none_or(|repositories| repositories.is_empty()),
            "{listed}"
        );
        let (status, denied) = self
            .call(
                "GetExecution",
                "",
                json!({"repositoryId":rid,"executionId":"run"}),
            )
            .await;
        assert_ne!(status, StatusCode::OK, "{denied}");
        let (status, execution) = self
            .call(
                "GetExecution",
                "Bearer manager",
                json!({"repositoryId":rid,"executionId":"run"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{execution}");
        assert_eq!(
            execution["execution"]["measurements"][0]["rows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

async fn queued_repository_lock(
    f: &Fixture,
    rid: &str,
) -> (
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Notify>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let db = f.server.db.clone();
    let rid = uuid::Uuid::parse_str(rid).unwrap();
    let release = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    let (acquired, acquired_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tx = db.begin().await.unwrap();
        sqlx::query("SELECT id FROM git_tracker_repositories WHERE id=$1 FOR UPDATE")
            .bind(rid)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        acquired.send(()).unwrap();
        held.notified().await;
        tx.rollback().await.unwrap();
    });
    f.wait_for_lock_waiter().await;
    (task, release, acquired_rx)
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_optional_publication_timeout_accounting_and_push_skip() {
    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap().to_owned();
    let (_, issued) = f
        .call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
        )
        .await;
    let auth = format!("Bearer {}", issued["token"].as_str().unwrap());
    let github_failures = f
        .server
        .metrics
        .events
        .with_label_values(&["github", "failure"]);
    let persistence_failures = f
        .server
        .metrics
        .events
        .with_label_values(&["persistence", "failure"]);
    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});

    // A queued writer receives the mandatory lock after the upload commits. A push response must
    // finish while that writer still holds it: publication must not acquire a second lock.
    for method in ["Upload", "Complete"] {
        let id = format!("push-{method}");
        let context =
            json!({"repositoryId":rid,"executionId":id,"commit":f.commit,"branch":"main"});
        let request = if method == "Upload" {
            json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]})
        } else {
            json!({"context":context})
        };
        let (entered, resume) = f.arm_resolve_gate();
        let http = f.http.clone();
        let endpoint = format!("{}/{method}", f.base);
        let auth = auth.clone();
        let response = tokio::spawn(async move {
            let response = http
                .post(endpoint)
                .header("Authorization", auth)
                .header("Connect-Protocol-Version", "1")
                .header("Connect-Timeout-Ms", "2500")
                .json(&request)
                .send()
                .await
                .unwrap();
            (response.status(), response.json::<Value>().await.unwrap())
        });
        entered.notified().await;
        let (lock, unlock, acquired) = queued_repository_lock(&f, &rid).await;
        resume.notify_one();
        tokio::time::timeout(Duration::from_secs(2), acquired)
            .await
            .unwrap()
            .unwrap();
        let (status, body) = tokio::time::timeout(Duration::from_millis(300), response)
            .await
            .expect("push attempted optional repository lock")
            .unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.get("warnings").is_none(), "{body}");
        assert_eq!(github_failures.get(), 0);
        assert_eq!(persistence_failures.get(), 0);
        unlock.notify_one();
        lock.await.unwrap();
    }

    for method in ["Upload", "Complete"] {
        let id = format!("push-expired-{method}");
        let context =
            json!({"repositoryId":rid,"executionId":id,"commit":f.commit,"branch":"main"});
        let request = if method == "Upload" {
            json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]})
        } else {
            json!({"context":context})
        };
        let response = f
            .http
            .post(format!("{}/{method}", f.base))
            .header("Authorization", &auth)
            .header("Connect-Protocol-Version", "1")
            .header("Connect-Timeout-Ms", "1500")
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert!(body.get("warnings").is_none(), "{body}");
        assert_eq!(github_failures.get(), 0);
        assert_eq!(persistence_failures.get(), 0);
    }

    let expired_pr = json!({"repositoryId":rid,"executionId":"pr-expired","commit":f.commit,"branch":"main","pullRequest":"1","baselineCommit":f.commit});
    let response = f
        .http
        .post(format!("{}/Upload", f.base))
        .header("Authorization", &auth)
        .header("Connect-Protocol-Version", "1")
        .header("Connect-Timeout-Ms", "1500")
        .json(&json!({"context":expired_pr,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["warnings"][0], "PR comment could not be updated");
    assert_eq!(github_failures.get(), 1);
    assert_eq!(persistence_failures.get(), 0);

    // Every run forces the optional PR publisher to wait behind the queued writer. Either the
    // PostgreSQL lock_timeout or the Tokio cutoff may win; both count only a GitHub failure.
    for i in 0..50 {
        let id = format!("pr-lock-{i}");
        let context = json!({"repositoryId":rid,"executionId":id,"commit":f.commit,"branch":"main","pullRequest":"1","baselineCommit":f.commit});
        let request = json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]});
        let (entered, resume) = f.arm_resolve_gate();
        let http = f.http.clone();
        let endpoint = format!("{}/Upload", f.base);
        let auth = auth.clone();
        let response = tokio::spawn(async move {
            let response = http
                .post(endpoint)
                .header("Authorization", auth)
                .header("Connect-Protocol-Version", "1")
                .header("Connect-Timeout-Ms", "3500")
                .json(&request)
                .send()
                .await
                .unwrap();
            (response.status(), response.json::<Value>().await.unwrap())
        });
        entered.notified().await;
        let (lock, unlock, acquired) = queued_repository_lock(&f, &rid).await;
        resume.notify_one();
        tokio::time::timeout(Duration::from_secs(2), acquired)
            .await
            .unwrap()
            .unwrap();
        f.wait_for_lock_waiter().await;
        let before = github_failures.get();
        let (status, body) = response.await.unwrap();
        assert_eq!(status, StatusCode::OK, "run {i}: {body}");
        assert_eq!(
            body["warnings"][0], "PR comment could not be updated",
            "run {i}: {body}"
        );
        assert_eq!(github_failures.get(), before + 1, "run {i}");
        assert_eq!(persistence_failures.get(), 0, "run {i}");
        let saved: bool = sqlx::query_scalar(&format!(
            "SELECT jsonb_exists(state->'records',$1) FROM {}.git_tracker_repositories WHERE id=$2",
            f.schema
        ))
        .bind(&id)
        .bind(uuid::Uuid::parse_str(&rid).unwrap())
        .fetch_one(&f.admin)
        .await
        .unwrap();
        assert!(saved, "run {i}");
        unlock.notify_one();
        lock.await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_genuine_publication_database_error_keeps_persistence_accounting() {
    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap().to_owned();
    let (_, issued) = f
        .call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
        )
        .await;
    let auth = format!("Bearer {}", issued["token"].as_str().unwrap());
    let context = json!({"repositoryId":rid,"executionId":"pr-database-failure","commit":f.commit,"branch":"main","pullRequest":"1","baselineCommit":f.commit});
    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let (entered, resume) = f.arm_resolve_gate();
    let http = f.http.clone();
    let endpoint = format!("{}/Upload", f.base);
    let response = tokio::spawn(async move {
        let response = http
            .post(endpoint)
            .header("Authorization", auth)
            .header("Connect-Protocol-Version", "1")
            .json(&json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}))
            .send()
            .await
            .unwrap();
        (response.status(), response.json::<Value>().await.unwrap())
    });
    entered.notified().await;
    let db = f.server.db.clone();
    let rename = tokio::spawn(async move {
        sqlx::query("ALTER TABLE git_tracker_repositories RENAME TO renamed_repositories")
            .execute(&db)
            .await
            .unwrap();
    });
    f.wait_for_lock_waiter().await;
    resume.notify_one();
    rename.await.unwrap();
    let (status, body) = response.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["warnings"][0], "PR comment could not be updated");
    assert_eq!(
        f.server
            .metrics
            .events
            .with_label_values(&["persistence", "failure"])
            .get(),
        1
    );
    assert_eq!(
        f.server
            .metrics
            .events
            .with_label_values(&["github", "failure"])
            .get(),
        0
    );
    let saved: bool = sqlx::query_scalar(&format!(
        "SELECT jsonb_exists(state->'records','pr-database-failure') FROM {}.renamed_repositories WHERE id=$1",
        f.schema
    ))
    .bind(uuid::Uuid::parse_str(&rid).unwrap())
    .fetch_one(&f.admin)
    .await
    .unwrap();
    assert!(saved);
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_space_id_validation_precedes_delcore_requests() {
    use std::sync::atomic::Ordering;

    let fixture = fixture().await;
    let invalid = [
        "x".to_owned(),
        fixture.space.to_uppercase(),
        "550e8400-e29b-41d4-a716-446655440000".to_owned(),
    ];
    for space in invalid {
        for (method, body) in [
            ("ListRepositories", json!({"spaceId":space})),
            (
                "ConnectRepository",
                json!({"spaceId":space,"fullName":"owner/repo","githubAccessToken":"fixture"}),
            ),
        ] {
            let before = fixture.delcore_requests.load(Ordering::SeqCst);
            let (status, response) = fixture.call(method, "Bearer manager", body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{method}: {response}");
            assert_eq!(response["code"], "invalid_argument", "{method}: {response}");
            assert_eq!(fixture.delcore_requests.load(Ordering::SeqCst), before);
        }
    }
    assert!(fixture.row().await.is_none());

    let (status, listed) = fixture
        .call(
            "ListRepositories",
            "Bearer manager",
            json!({"spaceId":fixture.space}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(fixture.delcore_requests.load(Ordering::SeqCst) > 0);
    let (status, connected) = fixture.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_repository_discovery_preserves_failure_classes() {
    use std::sync::atomic::Ordering;
    let f = fixture_with_github_timeout(Duration::from_millis(100)).await;
    let token = "synthetic-user-token-must-stay-private";
    let failures = f
        .server
        .metrics
        .events
        .with_label_values(&["github", "failure"]);

    let (_, invalid) = f.connect_with("foo", token).await;
    assert_eq!(invalid["code"], "invalid_argument", "{invalid}");

    for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
        f.github
            .user_status
            .store(status.as_u16(), Ordering::SeqCst);
        let (_, denied) = f.connect_with("owner/repo", token).await;
        assert_eq!(denied["code"], "permission_denied", "{denied}");
        assert!(!denied.to_string().contains(token));
        assert!(
            !denied
                .to_string()
                .contains("provider-body-must-stay-private")
        );
    }
    f.github.user_status.store(200, Ordering::SeqCst);

    for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
        f.github
            .repository_status
            .store(status.as_u16(), Ordering::SeqCst);
        let (_, denied) = f.connect_with("owner/repo", token).await;
        assert_eq!(denied["code"], "permission_denied", "{denied}");
    }
    f.github.repository_status.store(404, Ordering::SeqCst);
    let (_, missing) = f.connect_with("owner/repo", token).await;
    assert_eq!(missing["code"], "not_found", "{missing}");

    f.github.repository_status.store(503, Ordering::SeqCst);
    let before = failures.get();
    let (_, unavailable) = f.connect_with("owner/repo", token).await;
    assert_eq!(unavailable["code"], "unavailable", "{unavailable}");
    assert_eq!(failures.get(), before + 1);
    assert!(!unavailable.to_string().contains(token));
    assert!(
        !unavailable
            .to_string()
            .contains("provider-body-must-stay-private")
    );
    f.github.repository_status.store(200, Ordering::SeqCst);

    f.github.repository_delay_ms.store(250, Ordering::SeqCst);
    let before = failures.get();
    let (_, timeout) = f.connect_with("owner/repo", token).await;
    assert_eq!(timeout["code"], "unavailable", "{timeout}");
    assert_eq!(failures.get(), before + 1);
    f.github.repository_delay_ms.store(0, Ordering::SeqCst);

    f.github.repository_admin.store(false, Ordering::SeqCst);
    let (_, denied) = f.connect_with("owner/repo", token).await;
    assert_eq!(denied["code"], "permission_denied", "{denied}");
    assert_eq!(f.row().await, None);
    assert_eq!(f.audit_count().await, 0);
    f.github.repository_admin.store(true, Ordering::SeqCst);

    let (status, connected) = f.connect_with("owner/repo", token).await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let row = f.row().await;
    assert!(row.is_some());
    assert_eq!(f.audit_count().await, 1);

    f.github.fail_metadata.store(true, Ordering::SeqCst);
    let (_, denied) = f.connect_with("owner/repo", token).await;
    assert_eq!(denied["code"], "permission_denied", "{denied}");
    assert_eq!(f.row().await, row);
    assert_eq!(f.audit_count().await, 1);
}

fn webhook_event() -> (String, Vec<u8>) {
    let raw = serde_json::to_vec(
        &json!({"action":"privatized","repository":{"id":42},"installation":{"id":1}}),
    )
    .unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-only").unwrap();
    mac.update(&raw);
    (
        format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        raw,
    )
}

/// Which committed private update runs while a same-space connection waits for the row lock.
#[derive(Clone, Copy)]
enum Interleaving {
    Webhook,
    Maintenance,
}

async fn connect_does_not_restore_anonymous_access(interleaving: Interleaving) {
    use std::sync::atomic::Ordering;
    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    assert_eq!(connected["repository"]["public"], true);
    let rid = connected["repository"]["id"].as_str().unwrap().to_owned();
    f.upload(&rid).await;
    let (status, anonymous) = f
        .call(
            "GetExecution",
            "",
            json!({"repositoryId":rid,"executionId":"run"}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "public history must be anonymously readable: {anonymous}"
    );

    // The webhook or maintenance pass holds the row lock while it fetches metadata. GitHub still
    // reports the repository as public at this point, which is the connection's older observation.
    let (entered, release) = f.arm_gate();
    let server = f.server.clone();
    let update = tokio::spawn(async move {
        match interleaving {
            Interleaving::Webhook => {
                let (signature, raw) = webhook_event();
                server.webhook("repository", &signature, &raw).await
            }
            Interleaving::Maintenance => server.maintain().await,
        }
    });
    entered.notified().await;
    let reconnect = {
        let (http, base, space) = (f.http.clone(), f.base.clone(), f.space.clone());
        tokio::spawn(async move {
            let r = http
                .post(format!("{base}/ConnectRepository"))
                .header("Authorization", "Bearer manager")
                .header("Connect-Protocol-Version", "1")
                .json(
                    &json!({"spaceId":space,"fullName":"owner/repo","githubAccessToken":"fixture"}),
                )
                .send()
                .await
                .unwrap();
            (r.status(), r.json::<Value>().await.unwrap())
        })
    };
    f.wait_for_lock_waiter().await;
    f.github.private.store(true, Ordering::SeqCst);
    release.notify_one();
    update.await.unwrap().unwrap();
    let (status, reconnected) = reconnect.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{reconnected}");
    assert_eq!(reconnected["repository"]["id"], rid);
    assert_ne!(reconnected["repository"]["public"], true, "{reconnected}");
    assert_eq!(f.row().await, Some((rid.clone(), false, true)));
    f.assert_private_with_history(&rid).await;

    // A genuinely newer public observation enables public reads again on the same connection.
    f.github.private.store(false, Ordering::SeqCst);
    let (status, republic) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{republic}");
    assert_eq!(republic["repository"]["id"], rid);
    assert_eq!(republic["repository"]["public"], true);
    assert_eq!(f.row().await, Some((rid.clone(), true, true)));
    let (status, anonymous) = f
        .call(
            "GetExecution",
            "",
            json!({"repositoryId":rid,"executionId":"run"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{anonymous}");
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_connect_keeps_private_visibility_committed_by_webhook() {
    connect_does_not_restore_anonymous_access(Interleaving::Webhook).await;
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_connect_keeps_private_visibility_committed_by_maintenance() {
    connect_does_not_restore_anonymous_access(Interleaving::Maintenance).await;
}

async fn insert_maintenance_repository(
    f: &Fixture,
    id: uuid::Uuid,
    space: uuid::Uuid,
    state: Value,
) {
    sqlx::query("INSERT INTO git_tracker_repositories (id,github_id,space_id,full_name,installation_id,public,connected,state) VALUES ($1,$2,$3,'fixture/repo','1',false,false,$4)")
        .bind(id)
        .bind(i64::from(id.as_bytes()[15]) + 100)
        .bind(space)
        .bind(state)
        .execute(&f.server.db)
        .await
        .unwrap();
}

fn stale_maintenance_state() -> Value {
    use delino_protocol::gittracker::v1::{Execution, ExecutionState::EXECUTION_STATE_MEASURING};
    use git_tracker_api::measurement::{Baseline, Record, State};
    let stale = chrono::Utc::now() - chrono::Duration::hours(7);
    let record = Record {
        removed: Default::default(),
        execution: Execution {
            state: EXECUTION_STATE_MEASURING.into(),
            ..Default::default()
        },
        last_data: stale,
        cutoff: stale,
        ancestors: Vec::new(),
        base: Baseline::default(),
        push: false,
        baselines: Default::default(),
    };
    let mut state = State::default();
    state.records.insert("run".into(), record);
    serde_json::to_value(state).unwrap()
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_maintenance_isolates_corrupt_repository_and_expires_audit() {
    let f = fixture().await;
    let space = uuid::Uuid::now_v7();
    let ids = [
        uuid::Uuid::parse_str("019a0000-0000-7000-8000-000000000001").unwrap(),
        uuid::Uuid::parse_str("019a0000-0000-7000-8000-000000000002").unwrap(),
        uuid::Uuid::parse_str("019a0000-0000-7000-8000-000000000003").unwrap(),
    ];
    insert_maintenance_repository(&f, ids[0], space, json!("broken")).await;
    for id in &ids[1..] {
        insert_maintenance_repository(&f, *id, space, stale_maintenance_state()).await;
    }
    let audit_id = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO git_tracker_audit VALUES($1,$2,'fixture',$3,'2000-01-01T00:00:00Z')")
        .bind(audit_id)
        .bind(space)
        .bind(uuid::Uuid::now_v7())
        .execute(&f.server.db)
        .await
        .unwrap();
    let failures = f
        .server
        .metrics
        .events
        .with_label_values(&["persistence", "failure"]);
    let before = failures.get();
    for cycle in 1..=2 {
        let error = f.server.maintain().await.unwrap_err();
        assert!(error.message.contains("1 repositories"), "{error}");
        f.server.record_failure(&error);
        assert_eq!(failures.get(), before + cycle);
    }
    for id in &ids[1..] {
        let state: Value =
            sqlx::query_scalar("SELECT state FROM git_tracker_repositories WHERE id=$1")
                .bind(id)
                .fetch_one(&f.server.db)
                .await
                .unwrap();
        assert_eq!(
            state["records"]["run"]["execution"]["state"],
            "EXECUTION_STATE_INTERRUPTED"
        );
    }
    let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM git_tracker_audit WHERE id=$1")
        .bind(audit_id)
        .fetch_one(&f.server.db)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_maintenance_skips_repository_deleted_after_listing() {
    let f = fixture().await;
    let space = uuid::Uuid::now_v7();
    let first = uuid::Uuid::parse_str("019a0000-0000-7000-8000-000000000011").unwrap();
    let disappeared = uuid::Uuid::parse_str("019a0000-0000-7000-8000-000000000012").unwrap();
    for id in [first, disappeared] {
        insert_maintenance_repository(&f, id, space, json!({})).await;
    }
    let mut lock = f.server.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM git_tracker_repositories WHERE id=$1 FOR UPDATE")
        .bind(first)
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let server = f.server.clone();
    let cycle = tokio::spawn(async move { server.maintain().await });
    f.wait_for_lock_waiter().await;
    sqlx::query("DELETE FROM git_tracker_repositories WHERE id=$1")
        .bind(disappeared)
        .execute(&f.server.db)
        .await
        .unwrap();
    lock.commit().await.unwrap();
    cycle.await.unwrap().unwrap();
    assert_eq!(
        f.server
            .metrics
            .events
            .with_label_values(&["persistence", "failure"])
            .get(),
        0
    );
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_maintenance_isolates_finalized_space_batches() {
    let f = fixture().await;
    let mut spaces = Vec::new();
    for i in 1..=101 {
        let id = uuid::Uuid::parse_str(&format!("019a0000-0000-7000-8000-{i:012x}")).unwrap();
        let space = uuid::Uuid::parse_str(&format!("019b0000-0000-7000-8000-{i:012x}")).unwrap();
        insert_maintenance_repository(&f, id, space, json!({})).await;
        spaces.push(space.to_string());
    }
    *f.finalized.lock().unwrap() = spaces.clone();
    sqlx::query(&format!("CREATE FUNCTION fail_first_cleanup() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.space_id = '{}'::uuid THEN RAISE EXCEPTION 'fixture'; END IF; RETURN OLD; END $$", spaces[0]))
        .execute(&f.server.db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_first_cleanup BEFORE DELETE ON git_tracker_repositories FOR EACH ROW EXECUTE FUNCTION fail_first_cleanup()")
        .execute(&f.server.db)
        .await
        .unwrap();
    let error = f.server.maintain().await.unwrap_err();
    assert!(error.message.contains("1 other operations"), "{error}");
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM git_tracker_repositories")
        .fetch_one(&f.server.db)
        .await
        .unwrap();
    assert_eq!(remaining, 100);
    assert_eq!(
        f.server
            .metrics
            .events
            .with_label_values(&["persistence", "failure"])
            .get(),
        1
    );
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_connect_aborts_without_partial_state_when_locked_refresh_fails() {
    use std::sync::atomic::Ordering;
    let f = fixture().await;

    // A first connection whose refresh fails must create nothing.
    f.github.fail_metadata.store(true, Ordering::SeqCst);
    let (status, failed) = f.connect().await;
    assert_ne!(status, StatusCode::OK, "{failed}");
    assert_eq!(f.row().await, None);
    assert_eq!(f.audit_count().await, 0);
    let links: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {}.git_tracker_links",
        f.schema
    ))
    .fetch_one(&f.admin)
    .await
    .unwrap();
    assert_eq!(links, 0);

    // A reconnect of a disconnected repository must neither reconnect it nor fall back to the
    // public snapshot taken during discovery.
    f.github.fail_metadata.store(false, Ordering::SeqCst);
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap().to_owned();
    let (status, body) = f
        .call("Disconnect", "Bearer manager", json!({"repositoryId":rid}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let audits = f.audit_count().await;
    f.github.fail_metadata.store(true, Ordering::SeqCst);
    let (status, failed) = f.connect().await;
    assert_ne!(status, StatusCode::OK, "{failed}");
    assert_eq!(f.row().await, Some((rid, false, false)));
    assert_eq!(f.audit_count().await, audits);
}
