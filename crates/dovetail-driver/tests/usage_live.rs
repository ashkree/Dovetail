//! Both drivers' real usage reports. Opt-in, though neither makes a model call:
//! `DOVETAIL_LIVE_TESTS=1 cargo test -p dovetail-driver --test usage_live -- --nocapture`.
//! Runs in a scratch folder under the target dir, with the user's own CLI accounts.

use dovetail_driver::{AgyDriver, ClaudeDriver, SessionDriver, UsageQuery};

fn query() -> UsageQuery {
    let scratch = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("usage-live");
    std::fs::create_dir_all(&scratch).expect("scratch");
    UsageQuery {
        cwd: scratch,
        env: Vec::new(),
    }
}

#[tokio::test]
async fn both_drivers_report_their_limits() {
    if std::env::var("DOVETAIL_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    let drivers: [Box<dyn SessionDriver>; 2] =
        [Box::new(ClaudeDriver::new()), Box::new(AgyDriver::new())];
    for driver in drivers {
        let usage = driver.usage(query()).await;
        println!("{usage:#?}");
        assert_eq!(usage.driver, driver.kind());
        assert!(!usage.failed, "{:?}", usage.note);
        assert!(!usage.groups.is_empty());
        assert!(usage.groups.iter().all(|g| !g.windows.is_empty()));
    }
}
