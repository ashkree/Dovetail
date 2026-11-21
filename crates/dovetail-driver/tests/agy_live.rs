//! One tiny real agy turn through `AgyDriver`. Opt-in, since it spends quota:
//! `DOVETAIL_LIVE_TESTS=1 cargo test -p dovetail-driver --test agy_live -- --nocapture`.
//! Runs in a scratch folder under the target dir with its own settings folder.

use dovetail_driver::{
    AgyDriver, DriverEvent, PermissionMode, SessionDriver, SessionSpec, UserInput,
};
use std::time::Duration;

#[tokio::test]
async fn a_real_agy_session_answers_and_switches_effort() {
    if std::env::var("DOVETAIL_LIVE_TESTS").as_deref() != Ok("1") {
        return;
    }
    let scratch = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("agy-live");
    let settings = scratch.join("settings");
    std::fs::create_dir_all(&settings).expect("scratch");
    std::fs::write(
        settings.join("settings.json"),
        r#"{"toolPermission":"request-review"}"#,
    )
    .expect("settings");
    let mut spec = SessionSpec::new("live", &scratch);
    spec.model = Some("gemini-3.8-flash".into());
    spec.effort = Some(dovetail_api::model::ReasoningEffort::Low);
    spec.permission_mode = PermissionMode::Manual;
    spec.agy.settings_dir = Some(settings);

    let mut session = AgyDriver::new().start(spec).expect("agy starts");
    let mut events = session.take_events();
    session
        .send(UserInput::text("Reply with the single word: pong"))
        .expect("send");
    let mut text = String::new();
    let mut agy_id = None;
    let run = async {
        while let Some(ev) = events.recv().await {
            match ev {
                DriverEvent::Init(init) => agy_id = Some(init.session_id),
                DriverEvent::TextDelta { text: t, .. } => text.push_str(&t),
                DriverEvent::TurnResult(r) => return Some(r),
                DriverEvent::Exited(e) => panic!("agy exited: {e:?}"),
                _ => {}
            }
        }
        None
    };
    let result = tokio::time::timeout(Duration::from_secs(120), run)
        .await
        .expect("a turn within 2 minutes")
        .expect("a result");
    println!("agy session {agy_id:?}: {text:?} ({})", result.subtype);
    assert_eq!(result.subtype, "success");
    assert!(text.to_lowercase().contains("pong"), "{text:?}");
    assert!(
        agy_id.is_some_and(|id| id != "live"),
        "agy picks its own id"
    );

    let switch = session
        .set_model(None, Some(dovetail_api::model::ReasoningEffort::Medium))
        .expect("switch");
    let confirmed = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(ev) = events.recv().await {
            if let DriverEvent::ModelSettings {
                switch: Some(s),
                effort,
                ..
            } = ev
                && s == switch
            {
                return effort;
            }
        }
        None
    })
    .await
    .expect("the switch is answered");
    assert_eq!(
        confirmed,
        Some(dovetail_api::model::ReasoningEffort::Medium)
    );
    session.shutdown(Duration::from_secs(5));
}
