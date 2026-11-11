// SPDX-License-Identifier: FSL-1.1-ALv2
use chrono::{Duration, Utc};
use delino_protocol::gittracker::v1::{ComparisonStatus::*, Direction::*, ExecutionState::*, *};
use git_tracker_api::{
    decimal,
    measurement::{State, validate},
};
fn column(unit: &str) -> Column {
    Column {
        id: "size".into(),
        unit: unit.into(),
        direction: DIRECTION_LOWER.into(),
        absolute_threshold: "0".into(),
        ..Default::default()
    }
}
fn dataset() -> Dataset {
    Dataset {
        id: "dataset".into(),
        name: "Dataset".into(),
        columns: vec![column("B")],
        ..Default::default()
    }
}
fn context(id: &str, commit: &str) -> Context {
    Context {
        repository_id: "repo".into(),
        execution_id: id.into(),
        commit: commit.into(),
        branch: "main".into(),
        ..Default::default()
    }
}
fn row(id: &str, value: &str) -> Row {
    Row {
        id: id.into(),
        values: [("size".into(), value.into())].into_iter().collect(),
        ..Default::default()
    }
}

#[test]
fn nul_input_is_rejected_by_measurement_validation_and_direct_state_calls() {
    for field in [
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
    ] {
        let mut dataset = dataset();
        let mut rows = vec![row("one", "1")];
        match field {
            "dataset ID" => dataset.id.push('\0'),
            "dataset name" => dataset.name.push('\0'),
            "column ID" => dataset.columns[0].id.push('\0'),
            "column name" => dataset.columns[0].name.push('\0'),
            "column unit" => dataset.columns[0].unit.push('\0'),
            "absolute threshold" => dataset.columns[0].absolute_threshold.push('\0'),
            "percentage threshold" => dataset.columns[0].percentage_threshold.push('\0'),
            "row ID" => rows[0].id.push('\0'),
            "row name" => rows[0].name.push('\0'),
            "row value column ID" => {
                rows[0].values.insert("size\0".into(), "1".into());
            }
            "row value" => *rows[0].values.get_mut("size").unwrap() = "1\0".into(),
            _ => unreachable!(),
        }
        let fault = validate(&dataset, &rows).unwrap_err();
        assert_eq!(fault.code, connectrpc::ErrorCode::FailedPrecondition);
        assert!(fault.message.contains(field), "{field}: {}", fault.message);
        assert!(!fault.message.contains('\0'));
        let mut state = State::default();
        let direct = state
            .upload(
                &context("run", "a"),
                &dataset,
                &rows,
                Utc::now(),
                vec![],
                String::new(),
            )
            .unwrap_err();
        assert_eq!(direct.message, fault.message);
        assert!(state.records.is_empty());
        assert_eq!(state.next, 0);
    }
    for field in [
        "repository ID",
        "execution ID",
        "commit",
        "branch",
        "baseline commit",
    ] {
        let mut context = context("run", "a");
        match field {
            "repository ID" => context.repository_id.push('\0'),
            "execution ID" => context.execution_id.push('\0'),
            "commit" => context.commit.push('\0'),
            "branch" => context.branch.push('\0'),
            "baseline commit" => context.baseline_commit.push('\0'),
            _ => unreachable!(),
        }
        let mut state = State::default();
        for fault in [
            state
                .upload(
                    &context,
                    &dataset(),
                    &[row("one", "1")],
                    Utc::now(),
                    vec![],
                    String::new(),
                )
                .unwrap_err(),
            state.complete(&context, Utc::now()).unwrap_err(),
        ] {
            assert_eq!(fault.code, connectrpc::ErrorCode::InvalidArgument);
            assert!(fault.message.contains(field), "{field}: {}", fault.message);
            assert!(!fault.message.contains('\0'));
        }
        assert!(state.records.is_empty());
        assert_eq!(state.next, 0);
    }
}

#[test]
fn explicit_comparison_prefers_branch_on_both_sides_in_either_registration_order() {
    for branch_first in [true, false] {
        let mut state = State::default();
        let at = Utc::now();
        let branch_dataset = dataset();
        let mut pull_request_dataset = dataset();
        pull_request_dataset.columns[0].absolute_threshold = "1000".into();
        for (commit, branch_value, pull_request_value) in
            [("target", "10", "99"), ("baseline", "4", "9")]
        {
            let branch = context(&format!("branch-{commit}"), commit);
            let mut pull_request = context(&format!("pr-{commit}"), commit);
            pull_request.pull_request = 1;
            let cases = if branch_first {
                [
                    (&branch, &branch_dataset, branch_value),
                    (&pull_request, &pull_request_dataset, pull_request_value),
                ]
            } else {
                [
                    (&pull_request, &pull_request_dataset, pull_request_value),
                    (&branch, &branch_dataset, branch_value),
                ]
            };
            for (execution, configuration, value) in cases {
                state
                    .upload(
                        execution,
                        configuration,
                        &[row("one", value)],
                        at,
                        vec![],
                        String::new(),
                    )
                    .unwrap();
                state.complete(execution, at).unwrap();
            }
        }
        let target = state.compare_representative("target").unwrap();
        let baseline = state.compare_representative("baseline").unwrap();
        assert_eq!(target.execution.context.execution_id, "branch-target");
        assert_eq!(baseline.execution.context.execution_id, "branch-baseline");
        let compared = &state.comparisons(target, Some(Some(baseline)))[0];
        assert_eq!(compared.value, "10");
        assert_eq!(compared.baseline_value, "4");
        assert_eq!(compared.baseline_execution_id, "branch-baseline");
        assert_eq!(compared.status, COMPARISON_STATUS_REGRESSION);
    }
}

#[test]
fn explicit_comparison_falls_back_to_latest_completed_pull_request() {
    let mut state = State::default();
    let at = Utc::now();
    for (id, value, completed) in [
        ("pr-old", "9", true),
        ("pr-new", "11", true),
        ("branch-incomplete", "20", false),
    ] {
        let mut execution = context(id, "shared");
        if id.starts_with("pr-") {
            execution.pull_request = 1;
        }
        state
            .upload(
                &execution,
                &dataset(),
                &[row("one", value)],
                at,
                vec![],
                String::new(),
            )
            .unwrap();
        if completed {
            state.complete(&execution, at).unwrap();
        }
    }
    assert_eq!(
        state
            .compare_representative("shared")
            .unwrap()
            .execution
            .context
            .execution_id,
        "pr-new"
    );
    assert!(state.compare_representative("missing").is_none());
}
#[test]
fn exact_decimals_thresholds_units_and_bounds() {
    assert_eq!(
        decimal::parse("1e3").unwrap(),
        decimal::parse("1000.000").unwrap()
    );
    for bad in [
        "NaN", "Infinity", "01", "1/2", "1e4097", "1e-4097", "+1", ".5", "1.",
    ] {
        assert!(decimal::parse(bad).is_err(), "{bad}")
    }
    let c = column("B");
    let v = decimal::compare(
        &c,
        Some(&c),
        "9007199254740993.00000000000001",
        "9007199254740993",
    );
    assert_eq!(v.status, COMPARISON_STATUS_REGRESSION);
    assert_eq!(v.delta, "0.00000000000001");
    assert_eq!(
        decimal::compare(&column("KiB"), Some(&c), "1", "1024").status,
        COMPARISON_STATUS_UNCHANGED
    );
    assert_eq!(
        decimal::compare(&column("ms"), Some(&c), "1", "1").status,
        COMPARISON_STATUS_INCOMPATIBLE
    );
    let mut c = column("B");
    c.absolute_threshold = "10".into();
    assert_eq!(
        decimal::compare(&c, Some(&c), "110", "100").status,
        COMPARISON_STATUS_CHANGED
    );
    assert_eq!(
        decimal::compare(&c, Some(&c), "110.0001", "100").status,
        COMPARISON_STATUS_REGRESSION
    );
    c.absolute_threshold.clear();
    c.percentage_threshold = "1".into();
    assert_eq!(
        decimal::compare(&c, Some(&c), "100", "0").status,
        COMPARISON_STATUS_CHANGED
    );
    c.direction = DIRECTION_NEUTRAL.into();
    assert_eq!(
        decimal::compare(&c, Some(&c), "1000", "1").status,
        COMPARISON_STATUS_CHANGED
    );
}
#[test]
fn incompatible_comparison_keeps_original_baseline_fields() {
    let mut s = State::default();
    let at = Utc::now();
    let base = context("base", "a");
    s.upload(
        &base,
        &dataset(),
        &[row("one", "1024")],
        at,
        vec![],
        String::new(),
    )
    .unwrap();
    s.complete(&base, at).unwrap();

    let mut target_dataset = dataset();
    target_dataset.columns[0].unit = "ms".into();
    let target = context("target", "b");
    s.upload(
        &target,
        &target_dataset,
        &[row("one", "2")],
        at + Duration::seconds(1),
        vec!["a".into()],
        String::new(),
    )
    .unwrap();

    let comparison = &s.comparisons(&s.records["target"], None)[0];
    assert_eq!(comparison.status, COMPARISON_STATUS_INCOMPATIBLE);
    assert_eq!(comparison.baseline_value, "1024");
    assert_eq!(comparison.baseline_unit, "B");
    assert_eq!(comparison.baseline_execution_id, "base");
    assert_eq!(comparison.delta, "");
    assert_eq!(comparison.percentage, "");
    let json = serde_json::to_value(comparison).unwrap();
    assert_eq!(json["baselineValue"], "1024");
    assert_eq!(json["baselineUnit"], "B");

    let compatible = decimal::compare(&column("KiB"), Some(&column("B")), "1", "1024");
    assert_eq!(compatible.status, COMPARISON_STATUS_UNCHANGED);
    assert_eq!(compatible.baseline_value, "1024");
    assert_eq!(compatible.baseline_unit, "B");
    assert_eq!(compatible.delta, "0");

    let new = decimal::compare(&column("ms"), None, "2", "");
    assert_eq!(new.status, COMPARISON_STATUS_NEW);
    assert_eq!(new.baseline_value, "");
    assert_eq!(new.baseline_unit, "");

    let unavailable = &s.comparisons(&s.records["target"], Some(None))[0];
    assert_eq!(unavailable.status, COMPARISON_STATUS_UNAVAILABLE);
    assert_eq!(unavailable.baseline_value, "");
    assert_eq!(unavailable.baseline_unit, "");

    s.records.remove("base");
    let deleted = &s.comparisons(&s.records["target"], None)[0];
    assert_eq!(deleted.status, COMPARISON_STATUS_BASELINE_DELETED);
    assert_eq!(deleted.baseline_value, "");
    assert_eq!(deleted.baseline_unit, "");
}
#[test]
fn atomic_upload_retry_lifecycle_and_registration_order() {
    let mut s = State::default();
    let at = Utc::now();
    let c = context("first", "a");
    let d = dataset();
    s.upload(&c, &d, &[row("one", "1")], at, vec![], String::new())
        .unwrap();
    assert!(
        s.upload(
            &c,
            &d,
            &[row("two", "2"), row("one", "3")],
            at,
            vec![],
            String::new()
        )
        .is_err()
    );
    assert_eq!(s.records["first"].execution.measurements[0].rows.len(), 1);
    s.upload(
        &c,
        &d,
        &[row("one", "1.00")],
        at + Duration::hours(5),
        vec![],
        String::new(),
    )
    .unwrap();
    assert_eq!(s.records["first"].last_data, at);
    let newer = context("second", "a");
    s.complete(&newer, at + Duration::minutes(1)).unwrap();
    s.complete(&c, at + Duration::minutes(2)).unwrap();
    assert_eq!(
        s.representative("a", None, false)
            .unwrap()
            .execution
            .context
            .execution_id,
        "second"
    );
    let completed = s.records["first"].execution.completed_at.clone();
    s.complete(&c, at + Duration::hours(9)).unwrap();
    assert_eq!(s.records["first"].execution.completed_at, completed);
    assert!(
        s.upload(&c, &d, &[row("one", "1")], at, vec![], String::new())
            .is_err()
    );
    let c = context("interrupted", "b");
    s.upload(&c, &d, &[row("one", "1")], at, vec![], String::new())
        .unwrap();
    assert_eq!(s.interrupt(at + Duration::hours(6)), 1);
    assert_eq!(
        s.records["interrupted"].execution.state,
        EXECUTION_STATE_INTERRUPTED
    );
    assert!(s.complete(&c, at + Duration::hours(6)).is_err());
}
#[test]
fn frozen_row_baselines_exclude_forks_and_survive_deletion() {
    let mut s = State::default();
    let at = Utc::now();
    let d = dataset();
    let base = context("base", "a");
    s.upload(
        &base,
        &d,
        &[row("one", "10"), row("two", "20")],
        at,
        vec![],
        String::new(),
    )
    .unwrap();
    s.complete(&base, at).unwrap();
    let mut fork = context("fork", "a");
    fork.pull_request = 1;
    s.upload(&fork, &d, &[row("one", "10000")], at, vec![], String::new())
        .unwrap();
    s.complete(&fork, at).unwrap();
    let target = context("target", "b");
    s.upload(
        &target,
        &d,
        &[row("one", "11")],
        at + Duration::seconds(1),
        vec!["a".into()],
        String::new(),
    )
    .unwrap();
    let later = context("late", "a");
    s.upload(
        &later,
        &d,
        &[row("two", "999")],
        at + Duration::seconds(2),
        vec![],
        String::new(),
    )
    .unwrap();
    s.complete(&later, at + Duration::seconds(2)).unwrap();
    s.upload(
        &target,
        &d,
        &[row("two", "21")],
        at + Duration::seconds(3),
        vec!["a".into()],
        String::new(),
    )
    .unwrap();
    let comparisons = s.comparisons(&s.records["target"], None);
    assert!(
        comparisons
            .iter()
            .all(|v| v.baseline_execution_id == "base")
    );
    s.records.remove("base");
    assert!(
        s.comparisons(&s.records["target"], None)
            .iter()
            .all(|v| v.status == COMPARISON_STATUS_BASELINE_DELETED)
    );
    s.deleted.insert("target".into(), true);
    s.records.remove("target");
    assert!(
        s.upload(&target, &d, &[row("one", "11")], at, vec![], String::new())
            .is_err()
    );
    assert!(s.complete(&target, at).is_err());
}
#[test]
fn current_storage_rows_round_trip() {
    let raw = serde_json::json!({"next":1,"records":{"run":{"execution":{"context":{"repositoryId":"r","executionId":"run","commit":"a","branch":"main","pullRequest":"3"},"state":"EXECUTION_STATE_MEASURING","registeredAt":"2026-01-01T00:00:00Z","registrationOrder":"00000000000000000001","measurements":[{"dataset":{"id":"d","name":"D","columns":[{"id":"size","unit":"B","direction":"DIRECTION_LOWER"}]},"rows":[{"id":"r","values":{"CamelKey":"1"}}]}]},"last_data":"2026-01-01T00:00:00Z","cutoff":"2026-01-01T00:00:00Z","ancestors":[],"removed":{},"base":{"execution":"","commit":""},"push":false,"baselines":{}}}});
    let s: State = serde_json::from_value(raw).unwrap();
    let saved = serde_json::to_value(&s).unwrap();
    let e = &saved["records"]["run"]["execution"];
    assert_eq!(e["context"]["pullRequest"], "3");
    assert_eq!(e["state"], "EXECUTION_STATE_MEASURING");
    assert_eq!(
        e["measurements"][0]["dataset"]["columns"][0]["direction"],
        "DIRECTION_LOWER"
    );
    assert_eq!(e["measurements"][0]["rows"][0]["values"]["CamelKey"], "1");
    let restored: State = serde_json::from_value(saved).unwrap();
    assert_eq!(
        restored.records["run"].execution,
        s.records["run"].execution
    );
}
