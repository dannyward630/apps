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
    assert_eq!(failed["code"], "failed_precondition");
    assert_eq!(failed["message"], "upload requires at least one row");
    assert_eq!(github_failures.get(), before);
    let (_, failed) = call(
        &http,
        &base,
        "Upload",
        &auth,
        json!({"context":bad_commit,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]}),
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
    provider_requests: std::sync::atomic::AtomicU64,
    private: std::sync::atomic::AtomicBool,
    fail_metadata: std::sync::atomic::AtomicBool,
    user_status: std::sync::atomic::AtomicU16,
    repository_status: std::sync::atomic::AtomicU16,
    repository_admin: std::sync::atomic::AtomicBool,
    repository_delay_ms: std::sync::atomic::AtomicU64,
    membership_mode: std::sync::atomic::AtomicU8,
    capture_comments: std::sync::atomic::AtomicBool,
    comments: std::sync::Mutex<Vec<(String, String)>>,
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

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_pull_request_comment_renders_identifiers_literally_on_post_and_patch() {
    use std::sync::atomic::Ordering;

    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let repository_id = connected["repository"]["id"].as_str().unwrap();
    let (_, issued) = f
        .call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":repository_id,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
        )
        .await;
    let token = format!("Bearer {}", issued["token"].as_str().unwrap());
    f.github.capture_comments.store(true, Ordering::SeqCst);

    let dataset_id = "@org/team";
    let row_id = "[x](https://example.invalid/)![i](https://example.invalid/i.png) #1 owner/repo#1 :+1: x\\|y `z`";
    let column_id = "c|d";
    let dataset = json!({"id":dataset_id,"name":"Size","columns":[{"id":column_id,"unit":"B","direction":"DIRECTION_LOWER"}]});
    let context = json!({"repositoryId":repository_id,"executionId":"literal-comment","commit":f.commit,"branch":"main","pullRequest":"1","baselineCommit":f.commit});
    let (status, uploaded) = f
        .call(
            "Upload",
            &token,
            json!({"context":context,"dataset":dataset,"rows":[{"id":row_id,"values":{column_id:"1"}}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    assert!(uploaded.get("warnings").is_none(), "{uploaded}");

    let large_row_id = format!("{}END", "n".repeat(1 << 20));
    let (status, uploaded) = f
        .call(
            "Upload",
            &token,
            json!({"context":context,"dataset":dataset,"rows":[{"id":large_row_id,"values":{column_id:"2"}}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    assert!(uploaded.get("warnings").is_none(), "{uploaded}");

    let comments = f.github.comments.lock().unwrap().clone();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].0, "POST");
    assert_eq!(comments[1].0, "PATCH");
    for (_, body) in comments.iter() {
        assert!(body.starts_with("<!-- git-tracker -->\n### Git Tracker\n"));
        assert!(body.contains("[Execution details](https://git-tracker.delino.io/"));
        assert!(body.contains("| Dataset / item / column | Value | Change | Result |"));
        assert!(body.contains("`@org/team`"));
        assert!(body.contains("`c\\|d`"));
        assert!(body.contains("<!---->"));
        assert!(body.contains("owner/repo#1 :+1:"));
        assert!(!body.contains("More rows are available"));
    }
    assert!(comments[1].1.contains(&format!("`{}…`", "n".repeat(120))));
    assert!(comments[1].1.len() < 60_000);
    let (status, details) = f
        .call(
            "GetExecution",
            "",
            json!({"repositoryId":repository_id,"executionId":"literal-comment"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{details}");
    let measurement = &details["execution"]["measurements"][0];
    assert_eq!(measurement["dataset"]["id"], dataset_id);
    let rows = measurement["rows"].as_array().unwrap();
    assert!(rows.iter().any(|row| row["id"] == row_id));
    assert!(rows.iter().any(|row| row["id"] == large_row_id));
}

async fn fixture() -> Fixture {
    fixture_with_github_timeout(Duration::from_secs(20)).await
}

async fn fixture_with_github_timeout(github_timeout: Duration) -> Fixture {
    fixture_with_deadlines(github_timeout, git_tracker_api::github::MEMBERSHIP_DEADLINE).await
}

async fn fixture_with_deadlines(
    github_timeout: Duration,
    membership_deadline: Duration,
) -> Fixture {
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
        provider_requests: 0.into(),
        private: false.into(),
        fail_metadata: false.into(),
        user_status: 200.into(),
        repository_status: 200.into(),
        repository_admin: true.into(),
        repository_delay_ms: 0.into(),
        membership_mode: 0.into(),
        capture_comments: false.into(),
        comments: std::sync::Mutex::new(Vec::new()),
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
            } else {
                github.provider_requests.fetch_add(1, Ordering::SeqCst);
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
                "/repos/owner/repo/pulls/1/commits" if github.membership_mode.load(Ordering::SeqCst) == 5 => 502,
                _ => 200,
            };
            if status != 200 {
                return (
                    StatusCode::from_u16(status).unwrap(),
                    Json(json!({"message":"provider-body-must-stay-private"})),
                );
            }
            if github.capture_comments.load(Ordering::SeqCst) {
                if p == "/repos/owner/repo/issues/1/comments"
                    && r.method() == axum::http::Method::GET
                {
                    let comments = github.comments.lock().unwrap();
                    let out = if comments.is_empty() {
                        json!([])
                    } else {
                        json!([{"id":1,"body":"<!-- git-tracker -->","performed_via_github_app":{"id":1}}])
                    };
                    return (StatusCode::OK, Json(out));
                }
                if (p == "/repos/owner/repo/issues/1/comments"
                    && r.method() == axum::http::Method::POST)
                    || (p == "/repos/owner/repo/issues/comments/1"
                        && r.method() == axum::http::Method::PATCH)
                {
                    let method = r.method().as_str().to_owned();
                    let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    github.comments.lock().unwrap().push((
                        method,
                        request["body"].as_str().unwrap().to_owned(),
                    ));
                    return (StatusCode::OK, Json(json!({"id":1})));
                }
            }
            let private = github.private.load(Ordering::SeqCst);
            let membership_mode = github.membership_mode.load(Ordering::SeqCst);
            if p == "/graphql" && membership_mode == 9 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            if p == "/graphql" && membership_mode == 8 {
                let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                let second = !request["variables"]["after"].is_null();
                let head = if second { "d".repeat(40) } else { "b".repeat(40) };
                let nodes = if second { json!([]) } else { json!([{"commit":{"oid":"c".repeat(40)}}]) };
                let response = json!({
                    "data": {"repository": {
                        "databaseId": 42,
                        "pullRequest": {
                            "id": "fixture-pr", "number": 1, "state": "OPEN",
                            "baseRefName": "main", "headRefOid": head,
                            "commits": {
                                "totalCount": 1, "nodes": nodes,
                                "pageInfo": {"hasNextPage": !second, "endCursor": if second { Value::Null } else { json!("c1") }}
                            }
                        }
                    }}
                });
                return (StatusCode::OK, Json(response));
            }
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
                    json!({"base":{"ref":if membership_mode == 2 {"release"} else {"main"},"sha":if membership_mode == 10 {"b".repeat(40)} else {commit.clone()}},"head":{"sha":if membership_mode == 0 || membership_mode == 10 {commit.clone()} else {"b".repeat(40)},"repo":{"id":43}}})
                }
                p if membership_mode == 10 && p.starts_with("/repos/owner/repo/compare/") => json!({"merge_base_commit":{"sha":"b".repeat(40)}}),
                "/repos/owner/repo/pulls/1/commits" if membership_mode == 6 => json!([{"sha":null}]),
                "/repos/owner/repo/pulls/1/commits" if membership_mode != 0 => json!([]),
                "/graphql" if membership_mode == 1 || membership_mode == 7 => json!({"data":{"repository":{"databaseId":42,"pullRequest":{"id":"fixture-pr","number":1,"state":"OPEN","baseRefName":"main","headRefOid":"b".repeat(40),"commits":{"totalCount":if membership_mode == 7 {1} else {0},"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}),
                "/graphql" if membership_mode == 3 => json!({"errors":[{"message":"fixture provider failure"}]}),
                "/repos/owner/repo/git/ref/heads/main" => {
                    json!({"ref":"refs/heads/main","object":{"type":"commit","sha":commit}})
                }
                "/repos/owner/repo/commits" if membership_mode == 10 => {
                    json!([
                        {"sha":commit,"parents":[{"sha":"b".repeat(40)}]},
                        {"sha":"b".repeat(40),"parents":[]}
                    ])
                }
                p if p.starts_with("/repos/owner/repo/commits/") => {
                    let sha = p.rsplit('/').next().unwrap();
                    if membership_mode == 10 {
                        json!({"sha":sha,"parents":if sha == "c".repeat(40) {json!([{"sha":"b".repeat(40)},{"sha":commit}])} else if sha == commit {json!([{"sha":"b".repeat(40)}])} else {json!([])}})
                    } else {
                        json!({"sha":if membership_mode == 4 {sha.to_owned()} else {commit},"parents":[]})
                    }
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
        github: {
            let mut client = GitHub::new(
                github_http,
                base.clone(),
                "1".into(),
                EncodingKey::from_rsa_pem(KEY).unwrap(),
            );
            client.membership_deadline = membership_deadline;
            client
        },
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

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_pull_request_oidc_explicit_baseline_uses_connected_repository_identity() {
    use std::sync::atomic::Ordering;

    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let repository_id = connected["repository"]["id"].as_str().unwrap();
    f.github.membership_mode.store(10, Ordering::SeqCst);
    let (_, issued) = f
        .call(
            "IssueToken",
            "Bearer manager",
            json!({
                "repositoryId":repository_id,
                "expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()
            }),
        )
        .await;
    let repository_token = format!("Bearer {}", issued["token"].as_str().unwrap());
    let baseline = "b".repeat(40);
    let merge = "c".repeat(40);
    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let baseline_context = json!({"repositoryId":repository_id,"executionId":"baseline","commit":baseline,"branch":"main"});
    let (status, uploaded) = f.call("Upload", &repository_token, json!({
        "context":baseline_context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"10"}}]
    })).await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    let (status, completed) = f
        .call(
            "Complete",
            &repository_token,
            json!({"context":baseline_context}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{completed}");

    let now = chrono::Utc::now().timestamp();
    let mut claims = json!({
        "iss":"https://token.actions.githubusercontent.com",
        "aud":"https://git-tracker.api.delino.io",
        "exp":now+300,"nbf":now-10,
        "repository_id":"42","run_id":"1","run_attempt":"1",
        "sha":merge,"ref":"refs/pull/1/merge",
        "sub":"repo:owner/repo:pull_request","event_name":"pull_request"
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("fixture".into());
    let sign = |claims: &Value| {
        format!(
            "Actions {}",
            jsonwebtoken::encode(&header, claims, &EncodingKey::from_rsa_pem(KEY).unwrap())
                .unwrap()
        )
    };
    let mut context = json!({
        "repositoryId":repository_id,"executionId":"42/1/1","commit":f.commit,
        "branch":"main","pullRequest":"1","baselineCommit":baseline
    });
    let (status, uploaded) = f
        .call(
            "Upload",
            &sign(&claims),
            json!({
                "context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"12"}}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    assert_eq!(uploaded["comparisons"][0]["baselineCommit"], baseline);
    assert_eq!(
        uploaded["comparisons"][0]["baselineExecutionId"],
        "baseline"
    );
    let (status, saved) = f
        .call(
            "GetExecution",
            "",
            json!({
                "repositoryId":repository_id,"executionId":"42/1/1"
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["execution"]["context"]["baselineCommit"], baseline);
    assert_eq!(saved["comparisons"][0]["baselineCommit"], baseline);

    claims["repository_id"] = "43".into();
    claims["run_id"] = "2".into();
    context["executionId"] = "43/2/1".into();
    let (status, denied) = f
        .call(
            "Upload",
            &sign(&claims),
            json!({
                "context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"13"}}]
            }),
        )
        .await;
    assert!(!status.is_success(), "{denied}");
    assert_eq!(denied["code"], "permission_denied");
    let registered: bool = sqlx::query_scalar(&format!(
        "SELECT jsonb_exists(state->'records',$1) FROM {}.git_tracker_repositories WHERE id=$2",
        f.schema
    ))
    .bind("43/2/1")
    .bind(uuid::Uuid::parse_str(repository_id).unwrap())
    .fetch_one(&f.admin)
    .await
    .unwrap();
    assert!(!registered);

    for (rid, run) in [("42", "3"), ("43", "4")] {
        claims["repository_id"] = rid.into();
        claims["run_id"] = run.into();
        context["executionId"] = format!("{rid}/{run}/1").into();
        context["baselineCommit"] = "".into();
        let (status, uploaded) = f.call("Upload", &sign(&claims), json!({
            "context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"14"}}]
        })).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "credential repository {rid}: {uploaded}"
        );
    }

    claims["repository_id"] = "42".into();
    claims["run_id"] = "5".into();
    claims["sha"] = f.commit.clone().into();
    claims["ref"] = "refs/heads/main".into();
    claims["sub"] = "repo:owner/repo:ref:refs/heads/main".into();
    claims["event_name"] = "push".into();
    context["executionId"] = "42/5/1".into();
    context["pullRequest"] = "0".into();
    let (status, uploaded) = f
        .call(
            "Upload",
            &sign(&claims),
            json!({
                "context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"15"}}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");

    context["executionId"] = "token-pr".into();
    context["pullRequest"] = "1".into();
    context["baselineCommit"] = baseline.into();
    let (status, uploaded) = f
        .call(
            "Upload",
            &repository_token,
            json!({
                "context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"16"}}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_upload_precheck_keeps_client_faults_out_of_github() {
    use std::sync::atomic::Ordering;

    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap();
    let (_, issued) = f
        .call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
        )
        .await;
    let token = format!("Bearer {}", issued["token"].as_str().unwrap());
    let stored = || async {
        sqlx::query_scalar::<_, Value>(&format!(
            "SELECT state FROM {}.git_tracker_repositories WHERE id=$1",
            f.schema
        ))
        .bind(uuid::Uuid::parse_str(rid).unwrap())
        .fetch_one(&f.admin)
        .await
        .unwrap()
    };
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
    let context =
        |id: &str| json!({"repositoryId":rid,"executionId":id,"commit":f.commit,"branch":"main"});
    let dataset = json!({"id":"size","name":"Size","columns":[{
        "id":"bytes","unit":"B","direction":"DIRECTION_LOWER"
    }]});
    let rows = json!([{"id":"one","values":{"bytes":"1"}}]);
    let mut cases = Vec::new();
    let mut unsupported = dataset.clone();
    unsupported["columns"][0]["unit"] = "parsec".into();
    cases.push((
        "unsupported unit",
        unsupported,
        rows.clone(),
        "unsupported column unit",
    ));
    cases.push((
        "empty rows",
        dataset.clone(),
        json!([]),
        "upload requires at least one row",
    ));
    let mut duplicate_columns = dataset.clone();
    duplicate_columns["columns"]
        .as_array_mut()
        .unwrap()
        .push(dataset["columns"][0].clone());
    cases.push((
        "duplicate columns",
        duplicate_columns,
        rows.clone(),
        "column IDs must be unique and nonempty",
    ));
    cases.push((
        "duplicate rows",
        dataset.clone(),
        json!([rows[0], rows[0]]),
        "row IDs must be unique and nonempty",
    ));
    cases.push((
        "invalid decimal",
        dataset.clone(),
        json!([{"id":"one","values":{"bytes":"invalid"}}]),
        "row one: invalid decimal",
    ));
    let baseline = stored().await;
    let baseline_audit = f.audit_count().await;
    for (name, dataset, rows, message) in cases {
        let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
        let (status, body) = f
            .call(
                "Upload",
                &token,
                json!({"context":context(name),"dataset":dataset,"rows":rows}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert_eq!(body["code"], "failed_precondition", "{name}: {body}");
        assert_eq!(body["message"], message, "{name}: {body}");
        assert_eq!(
            f.github.provider_requests.load(Ordering::SeqCst),
            before_provider,
            "{name}"
        );
        assert_eq!(stored().await, baseline, "{name}");
        assert_eq!(f.audit_count().await, baseline_audit, "{name}");
        assert_eq!(github_failures.get(), 0, "{name}");
        assert_eq!(persistence_failures.get(), 0, "{name}");
    }

    let run = context("run");
    let request = json!({"context":run,"dataset":dataset,"rows":rows});
    let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
    let (status, body) = f.call("Upload", &token, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(f.github.provider_requests.load(Ordering::SeqCst) > before_provider);
    let saved = stored().await;
    assert_eq!(saved["next"], 1);
    let mut retransmission = request.clone();
    retransmission["rows"][0]["values"]["bytes"] = "1.0".into();
    let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
    let (status, body) = f.call("Upload", &token, retransmission).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        f.github.provider_requests.load(Ordering::SeqCst),
        before_provider
    );
    assert_eq!(stored().await, saved);

    let mut conflicts = Vec::new();
    let mut other_context = request.clone();
    other_context["context"]["branch"] = "other".into();
    conflicts.push(("context", other_context, "execution context conflicts"));
    let mut other_dataset = request.clone();
    other_dataset["dataset"]["name"] = "Other".into();
    conflicts.push(("dataset", other_dataset, "dataset configuration conflicts"));
    let mut other_value = request.clone();
    other_value["rows"][0]["values"]["bytes"] = "2".into();
    conflicts.push(("row", other_value.clone(), "row one: values conflict"));
    let mut multirow = other_value;
    multirow["rows"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({"id":"new","values":{"bytes":"3"}}));
    conflicts.push(("atomic multirow", multirow, "row one: values conflict"));
    for (name, request, message) in conflicts {
        let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
        let (status, body) = f.call("Upload", &token, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
        assert_eq!(body["message"], message, "{name}: {body}");
        assert_eq!(
            f.github.provider_requests.load(Ordering::SeqCst),
            before_provider,
            "{name}"
        );
        assert_eq!(stored().await, saved, "{name}");
        assert_eq!(f.audit_count().await, baseline_audit, "{name}");
        assert_eq!(github_failures.get(), 0, "{name}");
        assert_eq!(persistence_failures.get(), 0, "{name}");
    }

    let (status, body) = f.call("Complete", &token, json!({"context":run})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let completed = stored().await;
    let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
    let (_, body) = f.call("Upload", &token, request.clone()).await;
    assert_eq!(body["message"], "execution is closed");
    assert_eq!(
        f.github.provider_requests.load(Ordering::SeqCst),
        before_provider
    );
    assert_eq!(stored().await, completed);

    let open = json!({"context":context("open"),"dataset":dataset,"rows":rows});
    let (status, body) = f.call("Upload", &token, open.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let delete =
        |kind: &str, id: &str| json!({"repositoryId":rid,"kind":kind,"id":id,"confirmed":true});
    let (status, body) = f
        .call(
            "Delete",
            "Bearer manager",
            delete("DELETE_KIND_EXECUTION", "run"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let deleted_execution = stored().await;
    let deleted_audit = f.audit_count().await;
    let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
    let (_, body) = f.call("Upload", &token, request).await;
    assert_eq!(body["message"], "execution was deleted");
    assert_eq!(
        f.github.provider_requests.load(Ordering::SeqCst),
        before_provider
    );
    assert_eq!(stored().await, deleted_execution);
    assert_eq!(f.audit_count().await, deleted_audit);
    let (status, body) = f
        .call(
            "Delete",
            "Bearer manager",
            delete("DELETE_KIND_DATASET", "size"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let deleted_dataset = stored().await;
    let deleted_audit = f.audit_count().await;
    let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
    let (_, body) = f.call("Upload", &token, open).await;
    assert_eq!(body["message"], "dataset was deleted from this execution");
    assert_eq!(
        f.github.provider_requests.load(Ordering::SeqCst),
        before_provider
    );
    assert_eq!(stored().await, deleted_dataset);
    assert_eq!(f.audit_count().await, deleted_audit);

    let mut invalid_commit = context("invalid-commit");
    invalid_commit["commit"] = "b".repeat(40).into();
    let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
    let before_github_failure = github_failures.get();
    let (_, body) = f
        .call(
            "Upload",
            &token,
            json!({"context":invalid_commit,"dataset":dataset,"rows":rows}),
        )
        .await;
    assert_eq!(body["code"], "unavailable", "{body}");
    assert!(f.github.provider_requests.load(Ordering::SeqCst) > before_provider);
    assert_eq!(github_failures.get(), before_github_failure + 1);
    assert_eq!(persistence_failures.get(), 0);
    assert_eq!(stored().await, deleted_dataset);
    assert_eq!(f.audit_count().await, deleted_audit);
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_nul_uploads_and_completion_are_client_faults_without_persistence() {
    use std::sync::atomic::Ordering;

    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap();
    let (_, issued) = f
        .call(
            "IssueToken",
            "Bearer manager",
            json!({
                "repositoryId":rid,
                "expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()
            }),
        )
        .await;
    let token = format!("Bearer {}", issued["token"].as_str().unwrap());
    let stored = || async {
        sqlx::query_scalar::<_, Value>(&format!(
            "SELECT state FROM {}.git_tracker_repositories WHERE id=$1",
            f.schema
        ))
        .bind(uuid::Uuid::parse_str(rid).unwrap())
        .fetch_one(&f.admin)
        .await
        .unwrap()
    };
    let persistence_failures = f
        .server
        .metrics
        .events
        .with_label_values(&["persistence", "failure"]);
    let baseline_state = stored().await;
    let baseline_audit = f.audit_count().await;
    let baseline_failures = persistence_failures.get();

    for (index, field) in [
        "repository ID",
        "execution ID",
        "commit",
        "branch",
        "baseline commit",
        "dataset ID",
        "dataset name",
        "column ID",
        "column name",
        "column unit",
        "absolute threshold",
        "percentage threshold",
        "row ID",
        "row name",
        "row value column ID",
        "row value",
    ]
    .into_iter()
    .enumerate()
    {
        let mut context = json!({
            "repositoryId":rid,"executionId":format!("nul-{index}"),
            "commit":f.commit,"branch":"missing"
        });
        let mut dataset = json!({"id":"size","name":"Size","columns":[{
            "id":"bytes","name":"Bytes","unit":"B","direction":"DIRECTION_LOWER"
        }]});
        let mut rows = json!([{"id":"one","name":"One","values":{"bytes":"1"}}]);
        match field {
            "repository ID" => context["repositoryId"] = format!("{rid}\0").into(),
            "execution ID" => context["executionId"] = "run\0".into(),
            "commit" => context["commit"] = "a\0".into(),
            "branch" => context["branch"] = "main\0".into(),
            "baseline commit" => context["baselineCommit"] = "a\0".into(),
            "dataset ID" => dataset["id"] = "size\0".into(),
            "dataset name" => dataset["name"] = "Size\0".into(),
            "column ID" => dataset["columns"][0]["id"] = "bytes\0".into(),
            "column name" => dataset["columns"][0]["name"] = "Bytes\0".into(),
            "column unit" => dataset["columns"][0]["unit"] = "B\0".into(),
            "absolute threshold" => dataset["columns"][0]["absoluteThreshold"] = "1\0".into(),
            "percentage threshold" => dataset["columns"][0]["percentageThreshold"] = "1\0".into(),
            "row ID" => rows[0]["id"] = "one\0".into(),
            "row name" => rows[0]["name"] = "One\0".into(),
            "row value column ID" => rows[0]["values"]["bytes\0"] = "1".into(),
            "row value" => rows[0]["values"]["bytes"] = "1\0".into(),
            _ => unreachable!(),
        }
        let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
        let (status, body) = f
            .call(
                "Upload",
                &token,
                json!({
                    "context":context,"dataset":dataset,"rows":rows
                }),
            )
            .await;
        assert!(status.is_client_error(), "{field}: {status} {body}");
        assert_eq!(
            body["message"],
            format!("{field}: NUL character is not allowed")
        );
        assert_eq!(
            f.github.provider_requests.load(Ordering::SeqCst),
            before_provider,
            "{field}"
        );
        assert_eq!(stored().await, baseline_state, "{field}");
        assert_eq!(f.audit_count().await, baseline_audit, "{field}");
        assert_eq!(persistence_failures.get(), baseline_failures, "{field}");
    }

    for field in ["execution ID", "branch", "baseline commit"] {
        let mut context = json!({
            "repositoryId":rid,"executionId":"nul-complete","commit":f.commit,"branch":"missing"
        });
        match field {
            "execution ID" => context["executionId"] = "run\0".into(),
            "branch" => context["branch"] = "main\0".into(),
            "baseline commit" => context["baselineCommit"] = "a\0".into(),
            _ => unreachable!(),
        }
        let before_provider = f.github.provider_requests.load(Ordering::SeqCst);
        let (status, body) = f.call("Complete", &token, json!({"context":context})).await;
        assert!(status.is_client_error(), "{field}: {status} {body}");
        assert_eq!(
            body["message"],
            format!("{field}: NUL character is not allowed")
        );
        assert_eq!(
            f.github.provider_requests.load(Ordering::SeqCst),
            before_provider
        );
        assert_eq!(stored().await, baseline_state);
        assert_eq!(f.audit_count().await, baseline_audit);
        assert_eq!(persistence_failures.get(), baseline_failures);
    }

    let context =
        json!({"repositoryId":rid,"executionId":"unicode-run","commit":f.commit,"branch":"main"});
    let dataset = json!({"id":"größe-μs","name":"Size\u{1}","columns":[{
        "id":"bytes","name":"Bytes\u{3}","unit":"B","direction":"DIRECTION_LOWER"
    }]});
    let rows = json!([{"id":"élément","name":"One\u{2}","values":{"bytes":"1"}}]);
    let request = json!({"context":context,"dataset":dataset,"rows":rows});
    let (status, body) = f.call("Upload", &token, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let saved = stored().await;
    assert_eq!(saved["next"], 1);
    let (status, body) = f.call("Upload", &token, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored().await, saved);
    let mut changed = request;
    changed["rows"][0]["values"]["bytes"] = "2".into();
    let (status, body) = f.call("Upload", &token, changed).await;
    assert!(status.is_client_error(), "{body}");
    assert_eq!(body["code"], "failed_precondition");
    assert_eq!(stored().await, saved);
    let (status, body) = f.call("Complete", &token, json!({"context":context})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let completed = stored().await;
    let (status, body) = f.call("Complete", &token, json!({"context":context})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored().await, completed);
    assert_eq!(persistence_failures.get(), baseline_failures);
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_proven_upload_rejections_are_not_github_failures() {
    use std::sync::atomic::Ordering;
    let f = fixture_with_deadlines(Duration::from_secs(20), Duration::from_millis(500)).await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap();
    let (_, issued) = f.call("IssueToken", "Bearer manager", json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()})).await;
    let auth = format!("Bearer {}", issued["token"].as_str().unwrap());
    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let before_audit = f.audit_count().await;
    for (mode, branch, commit, code, github_failures) in [
        (1, "main", f.commit.clone(), "failed_precondition", 0),
        (2, "main", f.commit.clone(), "failed_precondition", 0),
        (4, "main", "b".repeat(40), "failed_precondition", 0),
        (3, "main", f.commit.clone(), "unavailable", 1),
        (5, "main", f.commit.clone(), "unavailable", 2),
        (6, "main", f.commit.clone(), "unavailable", 3),
        (7, "main", f.commit.clone(), "unavailable", 4),
        (8, "main", f.commit.clone(), "unavailable", 5),
        (9, "main", f.commit.clone(), "unavailable", 6),
    ] {
        f.github.membership_mode.store(mode, Ordering::SeqCst);
        let execution_id = format!("rejected-{mode}");
        let context = json!({"repositoryId":rid,"executionId":execution_id,"commit":commit,"branch":branch,"pullRequest":if mode == 4 {0} else {1}});
        let (status, body) = f.call("Upload", &auth, json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]})).await;
        assert!(!status.is_success(), "mode {mode}: {body}");
        assert_eq!(body["code"], code, "mode {mode}: {body}");
        assert_eq!(
            f.server
                .metrics
                .events
                .with_label_values(&["github", "failure"])
                .get(),
            github_failures,
            "mode {mode}"
        );
        let saved: bool = sqlx::query_scalar(&format!(
            "SELECT jsonb_exists(state->'records',$1) FROM {}.git_tracker_repositories WHERE id=$2",
            f.schema
        ))
        .bind(&execution_id)
        .bind(uuid::Uuid::parse_str(rid).unwrap())
        .fetch_one(&f.admin)
        .await
        .unwrap();
        assert!(!saved, "mode {mode}");
        assert_eq!(f.audit_count().await, before_audit, "mode {mode}");
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

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_revoke_token_requires_repository_token_match() {
    let fixture = fixture().await;
    let (status, connected) = fixture.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let repository_id = connected["repository"]["id"].as_str().unwrap();
    let foreign_repository_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO git_tracker_repositories(id,github_id,space_id,full_name,installation_id,public,connected) VALUES($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(foreign_repository_id)
    .bind(43_i64)
    .bind(uuid::Uuid::parse_str(&fixture.space).unwrap())
    .bind("owner/repo")
    .bind("1")
    .bind(true)
    .bind(true)
    .execute(&fixture.server.db)
    .await
    .unwrap();

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let (status, owned) = fixture
        .call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":repository_id,"expiresAt":expires_at}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{owned}");
    let (status, foreign) = fixture
        .call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":foreign_repository_id,"expiresAt":expires_at}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{foreign}");
    let revocations = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM git_tracker_audit WHERE action='token_revoked'",
        )
        .fetch_one(&fixture.server.db)
        .await
        .unwrap()
    };
    assert_eq!(revocations().await, 0);

    for (token_id, code) in [
        (foreign["tokenId"].as_str().unwrap().to_owned(), "not_found"),
        (uuid::Uuid::now_v7().to_string(), "not_found"),
        ("not-a-uuid".to_owned(), "invalid_argument"),
    ] {
        let (status, response) = fixture
            .call(
                "RevokeToken",
                "Bearer manager",
                json!({"repositoryId":repository_id,"tokenId":token_id}),
            )
            .await;
        assert_ne!(status, StatusCode::OK, "{response}");
        assert_eq!(response["code"], code, "{response}");
        assert_eq!(revocations().await, 0);
    }

    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let upload = |repository_id: &str, execution_id: &str| json!({"context":{"repositoryId":repository_id,"executionId":execution_id,"commit":fixture.commit,"branch":"main"},"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]});
    let foreign_auth = format!("Bearer {}", foreign["token"].as_str().unwrap());
    let (status, response) = fixture
        .call(
            "Upload",
            &foreign_auth,
            upload(&foreign_repository_id.to_string(), "foreign-still-valid"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let owned_id = owned["tokenId"].as_str().unwrap();
    for (call_index, token_id) in [owned_id.to_uppercase(), owned_id.to_owned()]
        .iter()
        .enumerate()
    {
        let (status, response) = fixture
            .call(
                "RevokeToken",
                "Bearer manager",
                json!({"repositoryId":repository_id,"tokenId":token_id}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(revocations().await, call_index as i64 + 1);
    }
    let revoked: bool = sqlx::query_scalar("SELECT revoked FROM git_tracker_tokens WHERE id=$1")
        .bind(uuid::Uuid::parse_str(owned_id).unwrap())
        .fetch_one(&fixture.server.db)
        .await
        .unwrap();
    assert!(revoked);
    let owned_auth = format!("Bearer {}", owned["token"].as_str().unwrap());
    let (status, denied) = fixture
        .call(
            "Upload",
            &owned_auth,
            upload(repository_id, "revoked-upload"),
        )
        .await;
    assert_ne!(status, StatusCode::OK, "{denied}");
    assert_eq!(denied["code"], "unauthenticated");
    assert_eq!(revocations().await, 2);
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
async fn postgres_delete_audits_only_existing_measurements() {
    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap();
    let initial_audits = f.audit_count().await;
    let delete =
        |kind: &str, id: &str| json!({"repositoryId":rid,"kind":kind,"id":id,"confirmed":true});
    let (status, body) = f
        .call(
            "Delete",
            "Bearer manager",
            delete("DELETE_KIND_REPOSITORY", ""),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.audit_count().await, initial_audits);

    for (kind, id) in [
        ("DELETE_KIND_EXECUTION", "missing"),
        ("DELETE_KIND_DATASET", "missing"),
    ] {
        let (_, body) = f.call("Delete", "Bearer manager", delete(kind, id)).await;
        assert_eq!(body["code"], "not_found", "{body}");
        assert_eq!(f.audit_count().await, initial_audits);
    }
    let state: Value = sqlx::query_scalar(&format!(
        "SELECT state FROM {}.git_tracker_repositories WHERE id=$1",
        f.schema
    ))
    .bind(uuid::Uuid::parse_str(rid).unwrap())
    .fetch_one(&f.admin)
    .await
    .unwrap();
    assert!(state["deleted"].as_object().unwrap().is_empty());

    let (_, issued) = f.call("IssueToken", "Bearer manager", json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()})).await;
    let token = format!("Bearer {}", issued["token"].as_str().unwrap());
    let context =
        json!({"repositoryId":rid,"executionId":"missing","commit":f.commit,"branch":"main"});
    let dataset = json!({"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let upload =
        json!({"context":context,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"1"}}]});
    let (status, body) = f.call("Upload", &token, upload.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = f
        .call(
            "Delete",
            "Bearer manager",
            delete("DELETE_KIND_EXECUTION", "missing"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.audit_count().await, initial_audits + 2); // Token issuance and actual deletion.
    let (status, body) = f
        .call(
            "Delete",
            "Bearer manager",
            delete("DELETE_KIND_EXECUTION", "missing"),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.audit_count().await, initial_audits + 2);
    let (_, body) = f.call("Upload", &token, upload).await;
    assert_eq!(body["code"], "failed_precondition", "{body}");
    let (_, body) = f.call("Complete", &token, json!({"context":context})).await;
    assert_eq!(body["code"], "failed_precondition", "{body}");

    let another =
        json!({"repositoryId":rid,"executionId":"another","commit":f.commit,"branch":"main"});
    let (status, body) = f.call("Upload", &token, json!({"context":another,"dataset":dataset,"rows":[{"id":"one","values":{"bytes":"2"}}]})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = f
        .call(
            "Delete",
            "Bearer manager",
            delete("DELETE_KIND_REPOSITORY", ""),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.audit_count().await, initial_audits + 3);
    let state: Value = sqlx::query_scalar(&format!(
        "SELECT state FROM {}.git_tracker_repositories WHERE id=$1",
        f.schema
    ))
    .bind(uuid::Uuid::parse_str(rid).unwrap())
    .fetch_one(&f.admin)
    .await
    .unwrap();
    assert!(state["records"].as_object().unwrap().is_empty());
    assert_eq!(state["deleted"]["another"], true);
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_delete_dataset_marks_holders_only_and_preserves_baselines() {
    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let rid = connected["repository"]["id"].as_str().unwrap();
    let (_, issued) = f.call("IssueToken", "Bearer manager", json!({"repositoryId":rid,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()})).await;
    let token = format!("Bearer {}", issued["token"].as_str().unwrap());
    let dataset = |id: &str| json!({"id":id,"name":id,"columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]});
    let context = |id: &str, pr: bool| json!({"repositoryId":rid,"executionId":id,"commit":f.commit,"branch":"main","pullRequest":if pr {1} else {0},"baselineCommit":if pr {f.commit.as_str()} else {""}});
    for (id, pr, data) in [
        ("baseline-new", false, "A"),
        ("pr-new", true, "A"),
        ("baseline-deleted", false, "D"),
        ("pr-deleted", true, "A"),
        ("nonholder", false, "A"),
    ] {
        let c = context(id, pr);
        let (status, body) = f.call("Upload", &token, json!({"context":c,"dataset":dataset(data),"rows":[{"id":"one","values":{"bytes":"1"}}]})).await;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
        if id.starts_with("baseline-") {
            let (status, body) = f.call("Complete", &token, json!({"context":c})).await;
            assert_eq!(status, StatusCode::OK, "{id}: {body}");
        }
    }
    let audits = f.audit_count().await;
    let delete = json!({"repositoryId":rid,"kind":"DELETE_KIND_DATASET","id":"D","confirmed":true});
    let (status, body) = f.call("Delete", "Bearer manager", delete.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.audit_count().await, audits + 1);
    let (_, body) = f.call("Delete", "Bearer manager", delete).await;
    assert_eq!(body["code"], "not_found", "{body}");
    assert_eq!(f.audit_count().await, audits + 1);

    let state: Value = sqlx::query_scalar(&format!(
        "SELECT state FROM {}.git_tracker_repositories WHERE id=$1",
        f.schema
    ))
    .bind(uuid::Uuid::parse_str(rid).unwrap())
    .fetch_one(&f.admin)
    .await
    .unwrap();
    assert_eq!(state["records"]["baseline-deleted"]["removed"]["D"], true);
    for id in ["baseline-new", "pr-new", "pr-deleted", "nonholder"] {
        assert!(state["records"][id]["removed"].get("D").is_none(), "{id}");
    }
    let (_, body) = f.call("Upload", &token, json!({"context":context("baseline-deleted", false),"dataset":dataset("D"),"rows":[{"id":"two","values":{"bytes":"2"}}]})).await;
    assert_eq!(body["code"], "failed_precondition", "{body}");
    for (id, expected) in [
        ("pr-new", "COMPARISON_STATUS_NEW"),
        ("pr-deleted", "COMPARISON_STATUS_BASELINE_DELETED"),
        ("nonholder", "COMPARISON_STATUS_NEW"),
    ] {
        let (status, body) = f.call("Upload", &token, json!({"context":context(id, id.starts_with("pr-")),"dataset":dataset("D"),"rows":[{"id":"one","values":{"bytes":"2"}}]})).await;
        assert_eq!(status, StatusCode::OK, "{id}: {body}");
        if id.starts_with("pr-") {
            assert_eq!(
                body["comparisons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["datasetId"] == "D")
                    .unwrap()["status"],
                expected,
                "{id}: {body}"
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_explicit_comparison_prefers_completed_branch_in_either_order() {
    for branch_first in [true, false] {
        let fixture = fixture().await;
        let (status, connected) = fixture.connect().await;
        assert_eq!(status, StatusCode::OK, "{connected}");
        let repository_id = connected["repository"]["id"].as_str().unwrap();
        let (_, issued) = fixture.call(
            "IssueToken",
            "Bearer manager",
            json!({"repositoryId":repository_id,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
        ).await;
        let token = format!("Bearer {}", issued["token"].as_str().unwrap());
        let cases = if branch_first {
            [("branch", 0, "10", "0"), ("pr", 1, "99", "1000")]
        } else {
            [("pr", 1, "99", "1000"), ("branch", 0, "10", "0")]
        };
        for (id, pull_request, value, threshold) in cases {
            let context = json!({
                "repositoryId":repository_id,
                "executionId":id,
                "commit":fixture.commit,
                "branch":"main",
                "pullRequest":pull_request,
                "baselineCommit":if pull_request > 0 { fixture.commit.as_str() } else { "" },
            });
            let (status, uploaded) = fixture.call("Upload", &token, json!({
                "context":context,
                "dataset":{"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER","absoluteThreshold":threshold}]},
                "rows":[{"id":"one","values":{"bytes":value}}],
            })).await;
            assert_eq!(status, StatusCode::OK, "{uploaded}");
            let (status, completed) = fixture
                .call("Complete", &token, json!({"context":context}))
                .await;
            assert_eq!(status, StatusCode::OK, "{completed}");
        }
        let (status, compared) = fixture
            .call(
                "Compare",
                "",
                json!({
                    "repositoryId":repository_id,
                    "targetCommit":fixture.commit,
                    "baselineCommit":fixture.commit,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{compared}");
        assert_eq!(compared["comparisons"][0]["value"], "10");
        assert_eq!(compared["comparisons"][0]["baselineValue"], "10");
        assert_eq!(compared["comparisons"][0]["baselineExecutionId"], "branch");

        let (status, history) = fixture
            .call(
                "GetHistory",
                "",
                json!({"repositoryId":repository_id,"branch":"main"}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{history}");
        let executions = history["executions"].as_array().unwrap();
        assert!(
            executions
                .iter()
                .any(|execution| execution["context"]["executionId"] == "branch"
                    && execution["representative"] == true)
        );
        assert!(
            executions
                .iter()
                .any(|execution| execution["context"]["executionId"] == "pr"
                    && execution["representative"] != true)
        );
    }
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_explicit_comparison_falls_back_to_latest_completed_pr() {
    let fixture = fixture().await;
    let (status, connected) = fixture.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    let repository_id = connected["repository"]["id"].as_str().unwrap();
    let (_, issued) = fixture.call(
        "IssueToken",
        "Bearer manager",
        json!({"repositoryId":repository_id,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339()}),
    ).await;
    let token = format!("Bearer {}", issued["token"].as_str().unwrap());
    for (id, pull_request, value, complete) in [
        ("pr-old", 1, "9", true),
        ("pr-new", 1, "11", true),
        ("branch-incomplete", 0, "20", false),
    ] {
        let context = json!({
            "repositoryId":repository_id,
            "executionId":id,
            "commit":fixture.commit,
            "branch":"main",
            "pullRequest":pull_request,
            "baselineCommit":if pull_request > 0 { fixture.commit.as_str() } else { "" },
        });
        let (status, uploaded) = fixture.call("Upload", &token, json!({
            "context":context,
            "dataset":{"id":"size","name":"Size","columns":[{"id":"bytes","unit":"B","direction":"DIRECTION_LOWER"}]},
            "rows":[{"id":"one","values":{"bytes":value}}],
        })).await;
        assert_eq!(status, StatusCode::OK, "{uploaded}");
        if complete {
            let (status, completed) = fixture
                .call("Complete", &token, json!({"context":context}))
                .await;
            assert_eq!(status, StatusCode::OK, "{completed}");
        }
    }
    let (status, compared) = fixture
        .call(
            "Compare",
            "",
            json!({
                "repositoryId":repository_id,
                "targetCommit":fixture.commit,
                "baselineCommit":fixture.commit,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{compared}");
    assert_eq!(compared["comparisons"][0]["value"], "11");
    assert_eq!(compared["comparisons"][0]["baselineValue"], "11");
    assert_eq!(compared["comparisons"][0]["baselineExecutionId"], "pr-new");
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_repository_listing_bounds_anonymous_results_and_preserves_space_results() {
    let fixture = fixture().await;
    let own_space = uuid::Uuid::parse_str(&fixture.space).unwrap();
    let other_space = uuid::Uuid::now_v7();
    let large_state = json!({"payload": "x".repeat(16_384)});
    let mut public_repositories = Vec::new();
    let mut own_repositories = Vec::new();

    for index in 0..153 {
        let id = uuid::Uuid::now_v7();
        let (space, full_name, public, connected) = match index {
            150 => (own_space, "owner/disconnected".to_owned(), true, false),
            151 => (own_space, "owner/private".to_owned(), false, true),
            152 => (other_space, "owner/repo-000".to_owned(), true, true),
            _ => (
                own_space,
                format!("owner/repo-{:03}", if index == 75 { 74 } else { index }),
                true,
                true,
            ),
        };
        sqlx::query(
            "INSERT INTO git_tracker_repositories \
             (id, github_id, space_id, full_name, installation_id, public, connected, state) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id)
        .bind(10_000_i64 + index)
        .bind(space)
        .bind(&full_name)
        .bind("fixture-installation")
        .bind(public)
        .bind(connected)
        .bind(&large_state)
        .execute(&fixture.server.db)
        .await
        .unwrap();
        if public && connected {
            public_repositories.push((full_name.clone(), id.to_string()));
        }
        if space == own_space {
            own_repositories.push((full_name, id.to_string()));
        }
    }
    public_repositories.sort();
    own_repositories.sort();

    let (status, anonymous) = fixture.call("ListRepositories", "", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{anonymous}");
    let anonymous_rows = anonymous["repositories"].as_array().unwrap();
    assert_eq!(anonymous_rows.len(), 100);
    let anonymous_identity: Vec<_> = anonymous_rows
        .iter()
        .map(|row| {
            assert!(row.get("installationId").is_none(), "{row}");
            assert_eq!(row["public"], true);
            assert_eq!(row["connected"], true);
            (
                row["fullName"].as_str().unwrap().to_owned(),
                row["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(anonymous_identity, public_repositories[..100]);

    let (status, authorized) = fixture
        .call(
            "ListRepositories",
            "Bearer manager",
            json!({"spaceId":fixture.space}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{authorized}");
    let authorized_rows = authorized["repositories"].as_array().unwrap();
    assert_eq!(authorized_rows.len(), 152);
    let authorized_identity: Vec<_> = authorized_rows
        .iter()
        .map(|row| {
            assert_eq!(row["installationId"], "fixture-installation");
            assert_eq!(row["spaceId"], fixture.space);
            (
                row["fullName"].as_str().unwrap().to_owned(),
                row["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(authorized_identity, own_repositories);
    assert!(authorized_rows.iter().any(|row| {
        row["fullName"] == "owner/disconnected"
            && !row
                .get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(false)
    }));
    assert!(authorized_rows.iter().any(|row| {
        row["fullName"] == "owner/private"
            && !row.get("public").and_then(Value::as_bool).unwrap_or(false)
    }));

    let (status, _) = fixture
        .call("ListRepositories", "", json!({"spaceId":fixture.space}))
        .await;
    assert_ne!(status, StatusCode::OK);
    let (status, malformed) = fixture
        .call(
            "ListRepositories",
            "Bearer manager",
            json!({"spaceId":"not-a-space"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{malformed}");
    assert_eq!(malformed["code"], "invalid_argument");
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

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn postgres_large_signed_webhook_respects_body_and_rpc_limits() {
    use server_common::http::Policy;

    let f = fixture().await;
    let (status, connected) = f.connect().await;
    assert_eq!(status, StatusCode::OK, "{connected}");
    for (github_id, installation) in [(43_i64, "2"), (44, "1")] {
        sqlx::query("INSERT INTO git_tracker_repositories(id,github_id,space_id,full_name,installation_id,public) VALUES($1,$2,$3,$4,$5,true)")
            .bind(uuid::Uuid::now_v7())
            .bind(github_id)
            .bind(uuid::Uuid::parse_str(&f.space).unwrap())
            .bind(format!("owner/repo{github_id}"))
            .bind(installation)
            .execute(&f.server.db)
            .await
            .unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/github/events", listener.local_addr().unwrap());
    let rpc_url = format!(
        "http://{}/gittracker.v1.GitTrackerService/ListRepositories",
        listener.local_addr().unwrap()
    );
    let rpc = connectrpc::Router::new()
        .add_service(f.server.clone())
        .into_axum_service()
        .with_limits(
            connectrpc::Limits::default()
                .with_max_message_size(8 << 20)
                .with_max_request_body_size(8 << 20),
        );
    let app = Router::new()
        .route(
            "/github/events",
            git_tracker_api::http::webhook_route(f.server.clone()),
        )
        .fallback_service(rpc)
        .layer(axum::middleware::from_fn_with_state(
            Policy {
                origin: String::new(),
                timeout: Duration::from_secs(60),
                service: Service::GitTracker,
                metrics: f.server.metrics.clone(),
                procedure: None,
            },
            server_common::http::middleware,
        ));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(65))
        .build()
        .unwrap();
    let signature = |body: &[u8]| {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-only").unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    };
    let state = |github_id| {
        sqlx::query_as::<_, (bool, bool)>(
            "SELECT connected,public FROM git_tracker_repositories WHERE github_id=$1",
        )
        .bind(github_id)
        .fetch_one(&f.server.db)
    };
    let before_audit = f.audit_count().await;
    let mut invalid =
        b"{\"action\":\"deleted\",\"installation\":{\"id\":1},\"padding\":\"".to_vec();
    invalid.resize((3 << 20) - 2, b'x');
    invalid.extend_from_slice(b"\"}");
    for supplied_signature in [None, Some("sha256=00")] {
        let mut request = client
            .post(&url)
            .header("X-GitHub-Event", "installation")
            .body(invalid.clone());
        if let Some(signature) = supplied_signature {
            request = request.header("X-Hub-Signature-256", signature);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().contains_key("X-Correlation-Id"));
        assert!(
            response
                .headers()
                .contains_key("Access-Control-Expose-Headers")
        );
        assert_eq!(state(42).await.unwrap(), (true, true));
        assert_eq!(f.audit_count().await, before_audit);
    }
    assert_eq!(
        client
            .post(&url)
            .body(vec![b'x'; (26 << 20) + 1])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(state(42).await.unwrap(), (true, true));

    let mut large =
        b"{\"action\":\"removed\",\"installation\":{\"id\":1},\"repositories_removed\":[".to_vec();
    for index in 0..100_000 {
        if index != 0 {
            large.push(b',');
        }
        let id = if index == 0 { 42 } else { 100_000 + index };
        large.extend_from_slice(format!("{{\"id\":{id}}}").as_bytes());
    }
    large.extend_from_slice(b"],\"padding\":\"");
    large.resize((20 << 20) - 2, b'x');
    large.extend_from_slice(b"\"}");
    assert_eq!(large.len(), 20 << 20);
    let started = std::time::Instant::now();
    let response = client
        .post(&url)
        .header("X-GitHub-Event", "installation_repositories")
        .header("X-Hub-Signature-256", signature(&large))
        .body(large)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(started.elapsed() < Duration::from_secs(60));
    assert_eq!(state(42).await.unwrap(), (false, false));
    assert_eq!(state(43).await.unwrap(), (true, true));
    assert_eq!(state(44).await.unwrap(), (true, true));
    assert_eq!(f.audit_count().await, before_audit + 1);
    assert_eq!(
        f.server
            .metrics
            .events
            .with_label_values(&["webhook", "success"])
            .get(),
        1
    );
    assert_eq!(
        f.server
            .metrics
            .events
            .with_label_values(&["webhook", "rejected"])
            .get(),
        3
    );
    let rpc_response = client
        .post(rpc_url)
        .header("Connect-Protocol-Version", "1")
        .header("Content-Type", "application/json")
        .body(format!("{{\"padding\":\"{}\"}}", "x".repeat((8 << 20) + 1)))
        .send()
        .await
        .unwrap();
    let rpc_body = rpc_response.text().await.unwrap();
    assert!(rpc_body.contains("resource_exhausted"), "{rpc_body}");
    task.abort();
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
