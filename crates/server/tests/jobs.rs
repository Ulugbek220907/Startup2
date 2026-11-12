//! The Postgres job queue: exclusivity, retries, lease recovery.

mod common;

use std::collections::HashSet;

use common::*;
use serde_json::json;

#[tokio::test]
async fn each_job_is_claimed_exactly_once_under_concurrency() {
    let app = spawn_app().await;
    let pool = &app.state.pool;
    for i in 0..40 {
        db::jobs::enqueue(pool, "test", &json!({ "n": i }), 0, 3).await.unwrap();
    }
    // Eight concurrent claimers race for the same rows. SKIP LOCKED means
    // nobody waits and nobody gets a duplicate.
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            let mut mine = Vec::new();
            while let Some(job) = db::jobs::claim(&pool, 60).await.unwrap() {
                mine.push(job.id);
                tokio::task::yield_now().await;
            }
            mine
        }));
    }
    let mut all = Vec::new();
    for t in tasks {
        all.extend(t.await.unwrap());
    }
    assert_eq!(all.len(), 40);
    assert_eq!(all.iter().collect::<HashSet<_>>().len(), 40, "a job was claimed twice");
}

#[tokio::test]
async fn failures_retry_with_backoff_then_give_up() {
    let app = spawn_app().await;
    let pool = &app.state.pool;
    let public = db::jobs::enqueue(pool, "test", &json!({}), 0, 2).await.unwrap();

    let job = db::jobs::claim(pool, 60).await.unwrap().unwrap();
    assert_eq!(job.attempts, 1);
    assert!(!db::jobs::fail(pool, &job, "boom", true).await.unwrap());
    // Back in the queue, but not due yet (30 s backoff).
    assert_eq!(db::jobs::status(pool, public).await.unwrap().as_deref(), Some("queued"));
    assert!(db::jobs::claim(pool, 60).await.unwrap().is_none());

    db::sqlx::query("UPDATE jobs SET run_at = now()").execute(pool).await.unwrap();
    let job = db::jobs::claim(pool, 60).await.unwrap().unwrap();
    assert_eq!(job.attempts, 2);
    assert!(db::jobs::fail(pool, &job, "boom again", true).await.unwrap(), "max attempts reached");
    assert_eq!(db::jobs::status(pool, public).await.unwrap().as_deref(), Some("failed"));

    // Non-retryable errors give up immediately.
    let public = db::jobs::enqueue(pool, "test", &json!({}), 0, 5).await.unwrap();
    let job = db::jobs::claim(pool, 60).await.unwrap().unwrap();
    assert!(db::jobs::fail(pool, &job, "bad payload", false).await.unwrap());
    assert_eq!(db::jobs::status(pool, public).await.unwrap().as_deref(), Some("failed"));
}

#[tokio::test]
async fn a_crashed_workers_job_is_picked_up_after_its_lease() {
    let app = spawn_app().await;
    let pool = &app.state.pool;
    db::jobs::enqueue(pool, "test", &json!({}), 0, 3).await.unwrap();
    let first = db::jobs::claim(pool, 60).await.unwrap().unwrap();
    // The worker "crashes": never completes. While leased, nobody else gets it.
    assert!(db::jobs::claim(pool, 60).await.unwrap().is_none());
    db::sqlx::query("UPDATE jobs SET locked_until = now() - interval '1 second'").execute(pool).await.unwrap();
    let second = db::jobs::claim(pool, 60).await.unwrap().unwrap();
    assert_eq!((second.id, second.attempts), (first.id, 2));
    db::jobs::complete(pool, second.id).await.unwrap();
    assert!(db::jobs::claim(pool, 60).await.unwrap().is_none());
}

#[tokio::test]
async fn delayed_jobs_wait_and_unknown_kinds_fail_cleanly() {
    let app = spawn_app().await;
    let pool = &app.state.pool;
    db::jobs::enqueue(pool, "test", &json!({}), 3600, 3).await.unwrap();
    assert!(db::jobs::claim(pool, 60).await.unwrap().is_none(), "a delayed job is not due yet");

    // The worker marks a job of an unknown kind failed without retrying.
    let public = db::jobs::enqueue(pool, "no_such_kind", &json!({}), 0, 5).await.unwrap();
    assert_eq!(app.drain_jobs().await, 1);
    assert_eq!(db::jobs::status(pool, public).await.unwrap().as_deref(), Some("failed"));
}

#[tokio::test]
async fn failed_render_marks_the_order_and_can_be_retried() {
    let app = spawn_app().await;
    let created = app.post_json("/api/orders", &order_body(&design_doc("Qayta", None), "901234567")).await.json();
    // Corrupt the frozen design so rendering cannot succeed.
    db::sqlx::query("UPDATE order_items SET design_doc = '{\"version\": 99}'::jsonb").execute(&app.state.pool).await.unwrap();
    app.drain_jobs().await;
    let status: String = db::sqlx::query_scalar("SELECT files_status FROM orders").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(status, "failed");

    // Fix the data, press "render again" in admin: files appear.
    db::sqlx::query("UPDATE order_items i SET design_doc = d.doc FROM designs d WHERE d.id = i.design_id").execute(&app.state.pool).await.unwrap();
    let admin = app.login().await;
    let id = app.admin_get(&admin, "/api/admin/orders").await.json()["orders"][0]["id"].as_str().unwrap().to_string();
    let r = app.admin_json(&admin, axum::http::Method::POST, &format!("/api/admin/orders/{id}/rerender"), &json!({})).await;
    assert_eq!(r.status, axum::http::StatusCode::OK);
    app.drain_jobs().await;
    let detail = app.admin_get(&admin, &format!("/api/admin/orders/{id}")).await.json();
    assert_eq!(detail["files_status"], "ready");
    assert_eq!(detail["row"]["code"], created["code"]);
}
