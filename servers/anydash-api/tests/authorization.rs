// SPDX-License-Identifier: FSL-1.1-ALv2
//! Tenant isolation, the team role matrix and data-owner sharing.
mod support;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use support::{Fixture, id};

fn layout(presentation: &str) -> Value {
    json!({"x":0,"y":0,"width":4,"height":2,"presentation":presentation})
}

struct AddWidgetRace {
    widget_type: &'static str,
    widget_layout: Value,
    query: Value,
    source: Value,
    deletion_query: &'static str,
    source_query: &'static str,
}

async fn version(f: &Fixture, caller: &support::Caller, dashboard: &str) -> String {
    f.ok(
        caller,
        "DashboardService/GetDashboard",
        json!({"dashboardId":dashboard}),
    )
    .await["dashboard"]["version"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn wait_for_lock_waits(f: &Fixture, fragments: &[&str]) {
    for _ in 0..100 {
        let mut all_waiting = true;
        for fragment in fragments {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() \
                 AND wait_event_type='Lock' AND position($1 in query)>0",
            )
            .bind(fragment)
            .fetch_one(&f.db)
            .await
            .unwrap();
            all_waiting &= count > 0;
        }
        if all_waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let activity: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT left(query,200),wait_event_type FROM pg_stat_activity WHERE datname=current_database()",
    )
    .fetch_all(&f.db)
    .await
    .unwrap();
    panic!("expected PostgreSQL lock waits for {fragments:?}: {activity:?}");
}

async fn wait_for_any_lock_wait(f: &Fixture, fragments: &[&str]) {
    for _ in 0..100 {
        for fragment in fragments {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() \
                 AND wait_event_type='Lock' AND position($1 in query)>0",
            )
            .bind(fragment)
            .fetch_one(&f.db)
            .await
            .unwrap();
            if count > 0 {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let activity: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT left(query,200),wait_event_type FROM pg_stat_activity WHERE datname=current_database()",
    )
    .fetch_all(&f.db)
    .await
    .unwrap();
    panic!("expected one PostgreSQL lock wait from {fragments:?}: {activity:?}");
}

async fn install_account_delete_barrier(f: &Fixture, table: &str) {
    f.exec(
        "CREATE OR REPLACE FUNCTION repair951_account_delete_barrier() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(901951); RETURN NEW; END $$",
    )
    .await;
    f.exec(&format!(
        "CREATE TRIGGER repair951_account_delete_barrier BEFORE UPDATE ON {table} \
         FOR EACH ROW WHEN (OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL) \
         EXECUTE FUNCTION repair951_account_delete_barrier()"
    ))
    .await;
}

async fn race_add_widget_with_account_deletion(
    f: &Arc<Fixture>,
    person: &support::Person,
    dashboard: &str,
    race: AddWidgetRace,
) {
    let hold = f.lock_row("SELECT pg_advisory_xact_lock(901951)").await;
    let deletion = {
        let f = Arc::clone(f);
        let person = person.clone();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "AccountService/DeleteAccountData",
                json!({"confirm":true}),
            )
            .await
        })
    };
    wait_for_lock_waits(f, &[race.deletion_query]).await;
    let addition = {
        let f = Arc::clone(f);
        let person = person.clone();
        let dashboard = dashboard.to_owned();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "DashboardService/AddWidget",
                json!({
                    "dashboardId": dashboard,
                    "expectedVersion": "1",
                    "requestId": id(),
                    "type": race.widget_type,
                    "layout": race.widget_layout,
                    "query": race.query,
                    "source": race.source
                }),
            )
            .await
        })
    };
    wait_for_lock_waits(f, &[race.source_query]).await;
    hold.commit().await.unwrap();

    tokio::time::timeout(Duration::from_secs(10), deletion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), addition)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(result.code, "not_found");
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widgets WHERE dashboard_id='{dashboard}'"
        ))
        .await,
        0
    );
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn account_deletion_and_add_widget_lock_connection_before_dashboard() {
    let f = Arc::new(Fixture::setup().await);
    let person = f.person("account-delete-connection-lock-owner");
    f.connect(&person).await;
    let (_, access, _) = f
        .enroll(&person, "account-delete-connection-device", "")
        .await;
    let connection = f
        .ok(
            &f.device(&access),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let caller = f.user(&person);
    let dashboard = f
        .ok(
            &caller,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":person.personal,"name":"Account deletion connection race"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    install_account_delete_barrier(&f, "connections").await;
    race_add_widget_with_account_deletion(
        &f,
        &person,
        &dashboard,
        AddWidgetRace {
            widget_type: "WIDGET_TYPE_SUBSCRIPTION_LIMITS",
            widget_layout: layout("WIDGET_PRESENTATION_GAUGE"),
            query: json!({}),
            source: json!({"connectionIds":[connection]}),
            deletion_query: "UPDATE connections SET deleted_at=COALESCE",
            source_query: "SELECT provider FROM connections WHERE id=ANY",
        },
    )
    .await;
    Arc::try_unwrap(f).ok().unwrap().cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn account_deletion_and_add_widget_lock_device_before_dashboard() {
    let f = Arc::new(Fixture::setup().await);
    let person = f.person("account-delete-device-lock-owner");
    f.connect(&person).await;
    let (device, _, _) = f.enroll(&person, "account-delete-device", "").await;
    let caller = f.user(&person);
    let dashboard = f
        .ok(
            &caller,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":person.personal,"name":"Account deletion device race"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    install_account_delete_barrier(&f, "devices").await;
    race_add_widget_with_account_deletion(
        &f,
        &person,
        &dashboard,
        AddWidgetRace {
            widget_type: "WIDGET_TYPE_LOCAL_USAGE_SERIES",
            widget_layout: layout("WIDGET_PRESENTATION_BAR_CHART"),
            query: json!({"period":"QUERY_PERIOD_LAST_7_DAYS","granularity":"TIME_GRANULARITY_DAY"}),
            source: json!({"deviceIds":[device]}),
            deletion_query: "UPDATE devices SET deleted_at=$2 WHERE user_id=$1",
            source_query: "SELECT id FROM devices WHERE id=ANY",
        },
    )
    .await;
    Arc::try_unwrap(f).ok().unwrap().cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn source_replacement_and_source_detachment_share_widget_first_lock_order() {
    let f = Arc::new(Fixture::setup().await);
    let person = f.person("source-replacement-lock-owner");
    f.connect(&person).await;
    let (_, access, _) = f
        .enroll(&person, "source-replacement-lock-device", "")
        .await;
    let caller = f.user(&person);
    let first = f
        .ok(
            &f.device(&access),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = f
        .ok(
            &f.device(&access),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let dashboard = f
        .ok(
            &caller,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":person.personal,"name":"Source replacement"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let widget = f.ok(&caller, "DashboardService/AddWidget",
        json!({"dashboardId":dashboard,"expectedVersion":"1","requestId":id(),"type":"WIDGET_TYPE_SUBSCRIPTION_LIMITS","layout":layout("WIDGET_PRESENTATION_GAUGE"),"query":{},"source":{"connectionIds":[first]}})).await["widgetId"].as_str().unwrap().to_owned();

    // Queue source deletion on the widget while source replacement is poised to change the
    // widget_sources row. With a consistent widget-before-source order, both transactions finish.
    let hold = f
        .lock_row(&format!(
            "SELECT id FROM widgets WHERE id='{widget}' FOR UPDATE"
        ))
        .await;
    let deletion = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let first = first.clone();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "ConnectionService/DeleteConnection",
                json!({"connectionId":first}),
            )
            .await
        })
    };
    wait_for_lock_waits(
        &f,
        &["SELECT w.id FROM widgets w JOIN widget_sources s ON s.widget_id=w.id"],
    )
    .await;
    let sharing = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let dashboard = dashboard.clone();
        let widget = widget.clone();
        let second = second.clone();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "DashboardService/SetWidgetSharingScope",
                json!({"dashboardId":dashboard,"expectedVersion":"2","widgetId":widget,"source":{"connectionIds":[second]}}),
            )
            .await
        })
    };
    wait_for_any_lock_wait(
        &f,
        &[
            "SELECT id FROM widgets WHERE id=$1 FOR UPDATE",
            "UPDATE widgets SET all_devices=$2 WHERE id=$1",
            "INSERT INTO widget_sources(widget_id,source_kind,source_id)",
        ],
    )
    .await;
    hold.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), deletion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), sharing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widgets WHERE id='{widget}' AND sharing_state=1"
        ))
        .await,
        1
    );
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widget_sources WHERE widget_id='{widget}' AND source_id='{second}'"
        ))
        .await,
        1
    );
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widget_sources WHERE widget_id='{widget}' AND source_id='{first}'"
        ))
        .await,
        0
    );
    Arc::try_unwrap(f).ok().unwrap().cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn source_detachment_and_widget_removal_share_widget_first_lock_order() {
    let f = Arc::new(Fixture::setup().await);
    let person = f.person("source-lock-owner");
    f.connect(&person).await;
    let (_, access, _) = f.enroll(&person, "source-lock-device", "").await;
    let caller = f.user(&person);
    let connection = f
        .ok(
            &f.device(&access),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let dashboard = f
        .ok(
            &caller,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":person.personal,"name":"Lock order"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let widget = f.ok(&caller, "DashboardService/AddWidget",
        json!({"dashboardId":dashboard,"expectedVersion":"1","requestId":id(),"type":"WIDGET_TYPE_SUBSCRIPTION_LIMITS","layout":layout("WIDGET_PRESENTATION_GAUGE"),"query":{},"source":{"connectionIds":[connection]}})).await["widgetId"].as_str().unwrap().to_owned();

    let hold = f
        .lock_row(&format!(
            "SELECT 1 FROM widget_sources WHERE widget_id='{widget}' FOR UPDATE"
        ))
        .await;
    let deletion = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let connection = connection.clone();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "ConnectionService/DeleteConnection",
                json!({"connectionId":connection}),
            )
            .await
        })
    };
    wait_for_lock_waits(&f, &["DELETE FROM widget_sources WHERE source_kind"]).await;
    let removal = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let dashboard = dashboard.clone();
        let widget = widget.clone();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "DashboardService/RemoveWidget",
                json!({"dashboardId":dashboard,"expectedVersion":"2","widgetId":widget}),
            )
            .await
        })
    };
    wait_for_lock_waits(
        &f,
        &[
            "DELETE FROM widget_sources WHERE source_kind",
            "DELETE FROM widgets WHERE id",
        ],
    )
    .await;
    hold.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), deletion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), removal)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        f.count(&format!("SELECT count(*) FROM widgets WHERE id='{widget}'"))
            .await,
        0
    );

    let device = f
        .count("SELECT count(*) FROM devices WHERE deleted_at IS NULL")
        .await;
    assert_eq!(device, 1);
    let device: String =
        sqlx::query_scalar("SELECT id::text FROM devices WHERE deleted_at IS NULL")
            .fetch_one(&f.db)
            .await
            .unwrap();
    let second_widget = f.ok(&caller, "DashboardService/AddWidget",
        json!({"dashboardId":dashboard,"expectedVersion":"3","requestId":id(),"type":"WIDGET_TYPE_LOCAL_USAGE_SERIES","layout":layout("WIDGET_PRESENTATION_BAR_CHART"),"query":{"period":"QUERY_PERIOD_LAST_7_DAYS","granularity":"TIME_GRANULARITY_DAY"},"source":{"deviceIds":[device]}})).await["widgetId"].as_str().unwrap().to_owned();
    f.ok(
        &caller,
        "DashboardService/DeleteDashboard",
        json!({"dashboardId":dashboard}),
    )
    .await;
    let hold = f
        .lock_row(&format!(
            "SELECT 1 FROM widget_sources WHERE widget_id='{second_widget}' FOR UPDATE"
        ))
        .await;
    let deletion = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let device = device.clone();
        tokio::spawn(async move {
            f.call(
                &f.user(&person),
                "DeviceService/DeleteDevice",
                json!({"deviceId":device}),
            )
            .await
        })
    };
    wait_for_lock_waits(&f, &["DELETE FROM widget_sources WHERE source_kind"]).await;
    let purge = {
        let f = Arc::clone(&f);
        let dashboard = uuid::Uuid::parse_str(&dashboard).unwrap();
        tokio::spawn(async move {
            let mut tx = f.db.begin().await.unwrap();
            anydash_api::worker::purge(
                &mut tx,
                anydash_api::rpc::DeletionKind::Dashboard as i16,
                Some(dashboard),
                None,
                None,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        })
    };
    wait_for_lock_waits(
        &f,
        &[
            "DELETE FROM widget_sources WHERE source_kind",
            "DELETE FROM dashboards WHERE id",
        ],
    )
    .await;
    hold.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), deletion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), purge)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widgets WHERE id='{second_widget}'"
        ))
        .await,
        0
    );
    Arc::try_unwrap(f).ok().unwrap().cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn source_validation_waits_for_device_deletion_and_rejects_it() {
    let f = Arc::new(Fixture::setup().await);
    let person = f.person("source-validation-owner");
    f.connect(&person).await;
    let (device, _, _) = f.enroll(&person, "source-validation-device", "").await;
    let caller = f.user(&person);
    let dashboard = f
        .ok(
            &caller,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":person.personal,"name":"Source validation"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut hold = f.db.begin().await.unwrap();
    sqlx::query("UPDATE devices SET deleted_at=now() WHERE id=$1")
        .bind(uuid::Uuid::parse_str(&device).unwrap())
        .execute(&mut *hold)
        .await
        .unwrap();
    let mut addition = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let dashboard = dashboard.clone();
        let device = device.clone();
        tokio::spawn(async move {
            f.call(&f.user(&person), "DashboardService/AddWidget",
            json!({"dashboardId":dashboard,"expectedVersion":"1","requestId":id(),"type":"WIDGET_TYPE_LOCAL_USAGE_SERIES","layout":layout("WIDGET_PRESENTATION_BAR_CHART"),"query":{"period":"QUERY_PERIOD_LAST_7_DAYS","granularity":"TIME_GRANULARITY_DAY"},"source":{"deviceIds":[device]}})).await
        })
    };
    tokio::select! {
        () = wait_for_lock_waits(&f, &["SELECT id FROM devices WHERE id=ANY"]) => {},
        result = &mut addition => panic!("AddWidget returned before source lock: {result:?}"),
    }
    hold.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), addition)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(result.code, "not_found");
    assert_eq!(f.count("SELECT count(*) FROM widgets").await, 0);

    let (_, access, _) = f.enroll(&person, "sharing-validation-device", "").await;
    let first = f
        .ok(
            &f.device(&access),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = f
        .ok(
            &f.device(&access),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let widget = f.ok(&caller, "DashboardService/AddWidget",
        json!({"dashboardId":dashboard,"expectedVersion":"1","requestId":id(),"type":"WIDGET_TYPE_SUBSCRIPTION_LIMITS","layout":layout("WIDGET_PRESENTATION_GAUGE"),"query":{},"source":{"connectionIds":[first]}})).await["widgetId"].as_str().unwrap().to_owned();
    let mut hold = f.db.begin().await.unwrap();
    sqlx::query("UPDATE connections SET deleted_at=now() WHERE id=$1")
        .bind(uuid::Uuid::parse_str(&second).unwrap())
        .execute(&mut *hold)
        .await
        .unwrap();
    let mut sharing = {
        let f = Arc::clone(&f);
        let person = person.clone();
        let dashboard = dashboard.clone();
        let widget = widget.clone();
        let second = second.clone();
        tokio::spawn(async move {
            f.call(&f.user(&person), "DashboardService/SetWidgetSharingScope",
            json!({"dashboardId":dashboard,"expectedVersion":"2","widgetId":widget,"source":{"connectionIds":[second]}})).await
        })
    };
    tokio::select! {
        () = wait_for_lock_waits(&f, &["SELECT provider FROM connections WHERE id=ANY"]) => {},
        result = &mut sharing => panic!("SetWidgetSharingScope returned before source lock: {result:?}"),
    }
    hold.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), sharing)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(result.code, "not_found");
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widget_sources WHERE widget_id='{widget}' AND source_id='{first}'"
        ))
        .await,
        1
    );
    f.ok(
        &caller,
        "ConnectionService/DeleteConnection",
        json!({"connectionId":first}),
    )
    .await;
    assert_eq!(f.count(&format!("SELECT count(*) FROM widgets WHERE id='{widget}' AND sharing_state=3 AND sharing_end_reason=3")).await, 1);
    Arc::try_unwrap(f).ok().unwrap().cleanup().await;
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn team_roles_isolation_and_data_owner_sharing() {
    let f = Fixture::setup().await;
    let owner = f.person("owner");
    let admin = f.person("admin");
    let member = f.person("member");
    let billing = f.person("billing");
    let outsider = f.person("outsider");
    let team = f.team(
        "Acme",
        &[
            (&owner, &["ROLE_OWNER"]),
            (&admin, &["ROLE_ADMIN"]),
            (&member, &["ROLE_MEMBER"]),
            (&billing, &["ROLE_BILLING"]),
        ],
    );
    for p in [&owner, &admin, &member, &billing, &outsider] {
        f.connect(p).await;
    }
    let (o, m, ad, bl, x) = (
        f.user(&owner),
        f.user(&member),
        f.user(&admin),
        f.user(&billing),
        f.user(&outsider),
    );
    let (_, owner_token, _) = f.enroll(&owner, "owner-laptop", "").await;
    let (member_device, member_token, _) = f.enroll(&member, "member-laptop", "").await;
    let od = f.device(&owner_token);
    let codex = f
        .ok(
            &od,
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Spaces: Members see the team with view and edit; Billing-only users do not see it.
    let spaces = |p: &support::Person| {
        let f = &f;
        let token = f.token(p, support::DELCORE_RESOURCE);
        let caller = f.user(p);
        async move {
            f.ok(
                &caller,
                "DashboardService/ListSpaces",
                json!({"delcoreAccessToken":token}),
            )
            .await
        }
    };
    let s = spaces(&member).await;
    let list = s["spaces"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["kind"], "SPACE_KIND_PERSONAL");
    assert_eq!(list[1]["id"], team);
    assert_eq!(
        list[1]["permissions"],
        json!([
            "SPACE_PERMISSION_VIEW_DASHBOARDS",
            "SPACE_PERMISSION_EDIT_DASHBOARDS"
        ])
    );
    assert_eq!(
        spaces(&billing).await["spaces"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        spaces(&admin).await["spaces"][1]["permissions"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    // The admin has no collector device: the team space works, the personal space needs onboarding.
    assert_eq!(
        spaces(&admin).await["spaces"][0]["onboardingState"],
        "ONBOARDING_STATE_DEVICE_REQUIRED"
    );
    f.fail(
        &ad,
        "DashboardService/ListDashboards",
        json!({"spaceId":admin.personal}),
        "ERROR_REASON_ONBOARDING_REQUIRED",
    )
    .await;
    f.ok(
        &ad,
        "DashboardService/ListDashboards",
        json!({"spaceId":team}),
    )
    .await;
    // A Delcore token of another subject is rejected.
    f.fail(
        &m,
        "DashboardService/ListSpaces",
        json!({"delcoreAccessToken": f.token(&owner, support::DELCORE_RESOURCE)}),
        "ERROR_REASON_UNAUTHENTICATED",
    )
    .await;

    // Create: Owner and Admin only.
    for (caller, reason) in [
        (&m, "ERROR_REASON_PERMISSION_DENIED"),
        (&bl, "ERROR_REASON_PERMISSION_DENIED"),
        (&x, "ERROR_REASON_PERMISSION_DENIED"),
    ] {
        f.fail(
            caller,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":team,"name":"Nope"}),
            reason,
        )
        .await;
    }
    let request = id();
    let dash = f
        .ok(
            &ad,
            "DashboardService/CreateDashboard",
            json!({"requestId":request,"spaceId":team,"name":"Team board"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let replay = f
        .ok(
            &ad,
            "DashboardService/CreateDashboard",
            json!({"requestId":request,"spaceId":team,"name":"Team board"}),
        )
        .await;
    assert_eq!(replay["dashboard"]["id"], dash);
    f.fail(
        &ad,
        "DashboardService/CreateDashboard",
        json!({"requestId":request,"spaceId":team,"name":"Other"}),
        "ERROR_REASON_REQUEST_ID_REUSED",
    )
    .await;
    // View: Owner, Admin and Member; Billing-only users and non-members never see it.
    for caller in [&o, &ad, &m] {
        f.ok(
            caller,
            "DashboardService/GetDashboard",
            json!({"dashboardId":dash}),
        )
        .await;
    }
    for caller in [&bl, &x] {
        f.fail(
            caller,
            "DashboardService/GetDashboard",
            json!({"dashboardId":dash}),
            "ERROR_REASON_NOT_FOUND",
        )
        .await;
        f.fail(
            caller,
            "DashboardService/ListDashboards",
            json!({"spaceId":team}),
            "ERROR_REASON_PERMISSION_DENIED",
        )
        .await;
    }
    // Members rename; versions are checked.
    let renamed = f
        .ok(
            &m,
            "DashboardService/RenameDashboard",
            json!({"dashboardId":dash,"expectedVersion":"1","name":"Renamed"}),
        )
        .await;
    assert_eq!(renamed["dashboard"]["version"], "2");
    let conflict = f
        .fail(
            &o,
            "DashboardService/RenameDashboard",
            json!({"dashboardId":dash,"expectedVersion":"1","name":"Stale"}),
            "ERROR_REASON_VERSION_CONFLICT",
        )
        .await;
    assert_eq!(conflict.detail.current_version, 2);
    assert_eq!(conflict.code, "aborted");

    // The owner shares subscription limits; the member shares local usage of one device.
    let owner_widget = f
        .ok(
            &o,
            "DashboardService/AddWidget",
            json!({"dashboardId":dash,"expectedVersion":"2","requestId":id(),"type":"WIDGET_TYPE_SUBSCRIPTION_LIMITS","layout":layout("WIDGET_PRESENTATION_GAUGE"),"query":{},"source":{"connectionIds":[codex]}}),
        )
        .await;
    let owner_widget_id = owner_widget["widgetId"].as_str().unwrap().to_owned();
    assert_eq!(
        owner_widget["dashboard"]["widgets"][0]["sharingState"],
        "SHARING_STATE_SHARED"
    );
    // A member cannot share another user's connection.
    f.fail(
        &m,
        "DashboardService/AddWidget",
        json!({"dashboardId":dash,"expectedVersion":"3","requestId":id(),"type":"WIDGET_TYPE_SUBSCRIPTION_LIMITS","layout":layout("WIDGET_PRESENTATION_GAUGE"),"query":{},"source":{"connectionIds":[codex]}}),
        "ERROR_REASON_NOT_FOUND",
    )
    .await;
    let member_widget = f
        .ok(
            &m,
            "DashboardService/AddWidget",
            json!({"dashboardId":dash,"expectedVersion":"3","requestId":id(),"type":"WIDGET_TYPE_LOCAL_USAGE_SERIES","layout":layout("WIDGET_PRESENTATION_BAR_CHART"),
                "query":{"period":"QUERY_PERIOD_LAST_7_DAYS","granularity":"TIME_GRANULARITY_DAY"},"source":{"deviceIds":[member_device]}}),
        )
        .await["widgetId"]
        .as_str()
        .unwrap()
        .to_owned();
    // Only the data owner sees the source.
    let seen = f
        .ok(
            &m,
            "DashboardService/GetDashboard",
            json!({"dashboardId":dash}),
        )
        .await;
    let widgets = seen["dashboard"]["widgets"].as_array().unwrap();
    let theirs = widgets.iter().find(|w| w["id"] == owner_widget_id).unwrap();
    assert!(theirs.get("source").is_none());
    assert_ne!(theirs["callerIsDataOwner"], true);
    let mine = widgets.iter().find(|w| w["id"] == member_widget).unwrap();
    assert_eq!(mine["callerIsDataOwner"], true);
    assert_eq!(mine["source"]["deviceIds"], json!([member_device]));
    // Members co-edit layout of any widget, but never another owner's query or sharing.
    let v = version(&f, &m, &dash).await;
    f.ok(
        &m,
        "DashboardService/UpdateWidgetLayout",
        json!({"dashboardId":dash,"expectedVersion":v,"updates":[{"widgetId":owner_widget_id,"layout":{"x":4,"y":1,"width":4,"height":3,"title":"Codex","presentation":"WIDGET_PRESENTATION_LIST"}}]}),
    )
    .await;
    let v = version(&f, &m, &dash).await;
    f.fail(
        &m,
        "DashboardService/UpdateWidgetQuery",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":owner_widget_id,"query":{"dimensions":["weekly"]}}),
        "ERROR_REASON_DATA_OWNER_REQUIRED",
    )
    .await;
    // A team role never bypasses the data-owner rule.
    f.fail(
        &ad,
        "DashboardService/SetWidgetSharingScope",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":owner_widget_id,"source":{"connectionIds":[codex]}}),
        "ERROR_REASON_DATA_OWNER_REQUIRED",
    )
    .await;
    f.fail(
        &ad,
        "DashboardService/RevokeWidgetSharing",
        json!({"dashboardId":dash,"widgetId":owner_widget_id}),
        "ERROR_REASON_DATA_OWNER_REQUIRED",
    )
    .await;
    // Members cannot remove another user's widget; Admins can.
    f.fail(
        &m,
        "DashboardService/RemoveWidget",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":owner_widget_id}),
        "ERROR_REASON_PERMISSION_DENIED",
    )
    .await;
    // Members cannot delete dashboards.
    f.fail(
        &m,
        "DashboardService/DeleteDashboard",
        json!({"dashboardId":dash}),
        "ERROR_REASON_PERMISSION_DENIED",
    )
    .await;
    // The owner updates their own query.
    let v = version(&f, &o, &dash).await;
    f.ok(
        &o,
        "DashboardService/UpdateWidgetQuery",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":owner_widget_id,"query":{"dimensions":["five_hour"]}}),
    )
    .await;

    // Shared data is readable by members; revocation ends access immediately.
    let query = |caller: &support::Caller, widget: &str| {
        let f = &f;
        let dash = dash.clone();
        let widget = widget.to_owned();
        let caller = support::Caller {
            authorization: caller.authorization.clone(),
            kind: caller.kind,
            version: caller.version,
        };
        async move {
            f.ok(
                &caller,
                "WidgetDataService/QueryWidgetData",
                json!({"dashboardId":dash,"widgetId":widget}),
            )
            .await
        }
    };
    let data = query(&m, &owner_widget_id).await;
    assert_eq!(data["data"]["sharingState"], "SHARING_STATE_SHARED");
    assert!(data["data"]["metricPanel"].is_object());
    let revoked = f
        .ok(
            &o,
            "DashboardService/RevokeWidgetSharing",
            json!({"dashboardId":dash,"widgetId":owner_widget_id}),
        )
        .await;
    assert!(revoked["dashboard"]["version"].is_string());
    let data = query(&m, &owner_widget_id).await;
    assert_eq!(data["data"]["sharingState"], "SHARING_STATE_ENDED");
    assert_eq!(
        data["data"]["sharingEndReason"],
        "SHARING_END_REASON_OWNER_REVOKED"
    );
    assert!(data["data"].get("metricPanel").is_none() && data["data"].get("summary").is_none());
    // Sharing again restores access.
    let v = version(&f, &o, &dash).await;
    f.ok(
        &o,
        "DashboardService/SetWidgetSharingScope",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":owner_widget_id,"source":{"connectionIds":[codex]}}),
    )
    .await;
    assert_eq!(
        query(&m, &owner_widget_id).await["data"]["sharingState"],
        "SHARING_STATE_SHARED"
    );
    // Leaving the team ends sharing immediately; rejoining does not restore it.
    f.set_roles(&team, &owner, &["ROLE_BILLING"]);
    let data = query(&m, &owner_widget_id).await;
    assert_eq!(data["data"]["sharingState"], "SHARING_STATE_ENDED");
    assert_eq!(
        data["data"]["sharingEndReason"],
        "SHARING_END_REASON_OWNER_LEFT_SPACE"
    );
    f.set_roles(&team, &owner, &["ROLE_OWNER"]);
    assert_eq!(
        query(&m, &owner_widget_id).await["data"]["sharingState"],
        "SHARING_STATE_ENDED"
    );
    // Role reads are batched: one call per tenant for every data owner of the dashboard.
    f.mock.lock().unwrap().calls.clear();
    f.ok(
        &ad,
        "WidgetDataService/QueryDashboardData",
        json!({"dashboardId":dash}),
    )
    .await;
    let roles_calls = f
        .mock
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|c| *c == "GetTenantMemberRoles")
        .count();
    assert_eq!(
        roles_calls, 2,
        "one for the caller and one batch for the data owners"
    );
    // Deleting the source device ends the member's widget.
    f.ok(
        &f.user(&member),
        "DeviceService/DeleteDevice",
        json!({"deviceId":member_device}),
    )
    .await;
    let data = query(&o, &member_widget).await;
    assert_eq!(
        data["data"]["sharingEndReason"],
        "SHARING_END_REASON_SOURCE_DELETED"
    );
    f.fail(
        &f.device(&member_token),
        "CollectorService/Heartbeat",
        json!({}),
        "ERROR_REASON_DEVICE_CREDENTIALS_INACTIVE",
    )
    .await;
    // Admins remove other users' widgets and delete dashboards.
    let v = version(&f, &ad, &dash).await;
    f.ok(
        &ad,
        "DashboardService/RemoveWidget",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":member_widget}),
    )
    .await;

    // Tenant isolation.
    f.fail(
        &x,
        "ConnectionService/GetConnection",
        json!({"connectionId":codex}),
        "ERROR_REASON_NOT_FOUND",
    )
    .await;
    f.fail(
        &x,
        "ConnectionService/DeleteConnection",
        json!({"connectionId":codex}),
        "ERROR_REASON_NOT_FOUND",
    )
    .await;
    assert_eq!(
        f.ok(&x, "DeviceService/ListDevices", json!({})).await["devices"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0),
        0
    );
    f.fail(
        &x,
        "DashboardService/ListDashboards",
        json!({"spaceId":owner.personal}),
        "ERROR_REASON_PERMISSION_DENIED",
    )
    .await;
    let personal = f
        .ok(
            &o,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":owner.personal,"name":"Mine"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for caller in [&x, &ad, &m] {
        f.fail(
            caller,
            "DashboardService/GetDashboard",
            json!({"dashboardId":personal}),
            "ERROR_REASON_NOT_FOUND",
        )
        .await;
        f.fail(
            caller,
            "WidgetDataService/QueryDashboardData",
            json!({"dashboardId":personal}),
            "ERROR_REASON_NOT_FOUND",
        )
        .await;
    }
    f.fail(
        &x,
        "DeviceService/SetPreferredDevice",
        json!({"deviceId":member_device}),
        "ERROR_REASON_NOT_FOUND",
    )
    .await;

    // Activity: Owners and Admins see team events; Members do not.
    let events = f
        .ok(&ad, "AccountService/ListActivity", json!({"spaceId":team}))
        .await;
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    for kind in [
        "ACTIVITY_KIND_DASHBOARD_CREATED",
        "ACTIVITY_KIND_WIDGET_SHARED",
        "ACTIVITY_KIND_WIDGET_SHARING_ENDED",
        "ACTIVITY_KIND_PERMISSION_DENIED",
    ] {
        assert!(kinds.contains(&kind), "{kind} in {kinds:?}");
    }
    f.fail(
        &m,
        "AccountService/ListActivity",
        json!({"spaceId":team}),
        "ERROR_REASON_PERMISSION_DENIED",
    )
    .await;
    let own = f.ok(&o, "AccountService/ListActivity", json!({})).await;
    let kinds: Vec<&str> = own["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"ACTIVITY_KIND_CONNECTION_CREATED")
            && kinds.contains(&"ACTIVITY_KIND_DEVICE_ENROLLED")
    );
    assert!(!kinds.contains(&"ACTIVITY_KIND_DASHBOARD_CREATED"));
    // Every denial wrote an audit event.
    assert!(
        f.count("SELECT count(*) FROM audit_events WHERE kind=16")
            .await
            >= 8
    );
    f.ok(
        &ad,
        "DashboardService/DeleteDashboard",
        json!({"dashboardId":dash}),
    )
    .await;
    f.fail(
        &m,
        "DashboardService/GetDashboard",
        json!({"dashboardId":dash}),
        "ERROR_REASON_NOT_FOUND",
    )
    .await;
    f.cleanup().await;
}

/// The widget's sharing state and end reason as reported by `QueryWidgetData`.
async fn sharing(
    f: &Fixture,
    caller: &support::Caller,
    dashboard: &str,
    widget: &str,
) -> (String, String) {
    let data = f
        .ok(
            caller,
            "WidgetDataService/QueryWidgetData",
            json!({"dashboardId":dashboard,"widgetId":widget}),
        )
        .await;
    (
        data["data"]["sharingState"].as_str().unwrap().to_owned(),
        data["data"]["sharingEndReason"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    )
}

#[tokio::test]
#[ignore = "requires a disposable TEST_DATABASE_URL"]
async fn departure_and_rejoin_end_sharing_without_an_intermediate_query() {
    let f = Fixture::setup().await;
    let owner = f.person("owner");
    let member = f.person("member");
    let team = f.team(
        "Acme",
        &[(&owner, &["ROLE_OWNER"]), (&member, &["ROLE_MEMBER"])],
    );
    for p in [&owner, &member] {
        f.connect(p).await;
    }
    let (o, m) = (f.user(&owner), f.user(&member));
    let (_, member_token, _) = f.enroll(&member, "member-laptop", "").await;
    let codex = f
        .ok(
            &f.device(&member_token),
            "ConnectionService/CreateConnection",
            json!({"requestId":id(),"provider":"PROVIDER_CODEX"}),
        )
        .await["connection"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let dash = f
        .ok(
            &o,
            "DashboardService/CreateDashboard",
            json!({"requestId":id(),"spaceId":team,"name":"Team board"}),
        )
        .await["dashboard"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let v = version(&f, &m, &dash).await;
    let widget = f
        .ok(
            &m,
            "DashboardService/AddWidget",
            json!({"dashboardId":dash,"expectedVersion":v,"requestId":id(),"type":"WIDGET_TYPE_SUBSCRIPTION_LIMITS","layout":layout("WIDGET_PRESENTATION_GAUGE"),"query":{},"source":{"connectionIds":[codex]}}),
        )
        .await["widgetId"]
        .as_str()
        .unwrap()
        .to_owned();
    // The data owner's membership start is stored with the shared widget.
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widgets WHERE id='{widget}' AND data_owner_member_since IS NOT NULL"
        ))
        .await,
        1
    );
    let state = |caller| sharing(&f, caller, &dash, &widget);
    assert_eq!(state(&o).await.0, "SHARING_STATE_SHARED");
    // A role change keeps the membership and therefore the sharing.
    f.set_roles(&team, &member, &["ROLE_ADMIN"]);
    assert_eq!(state(&o).await.0, "SHARING_STATE_SHARED");
    // The data owner leaves and rejoins with no query in between: the new membership start ends
    // sharing at the next evaluation, and the end is persisted.
    f.set_roles(&team, &member, &[]);
    f.set_roles(&team, &member, &["ROLE_MEMBER"]);
    assert_eq!(
        state(&o).await,
        (
            "SHARING_STATE_ENDED".to_owned(),
            "SHARING_END_REASON_OWNER_LEFT_SPACE".to_owned()
        )
    );
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM widgets WHERE id='{widget}' AND sharing_state=3 AND sharing_end_reason=2"
        ))
        .await,
        1
    );
    assert_eq!(state(&m).await.0, "SHARING_STATE_ENDED");
    // The rejoined owner can share explicitly again; the current membership start is stored.
    let v = version(&f, &m, &dash).await;
    f.ok(
        &m,
        "DashboardService/SetWidgetSharingScope",
        json!({"dashboardId":dash,"expectedVersion":v,"widgetId":widget,"source":{"connectionIds":[codex]}}),
    )
    .await;
    assert_eq!(state(&o).await.0, "SHARING_STATE_SHARED");
    assert_eq!(state(&m).await.0, "SHARING_STATE_SHARED");
    f.cleanup().await;
}
