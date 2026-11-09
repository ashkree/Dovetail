//! Image attachments go out as base64 image blocks in the stream-json user message.

use dovetail_driver::{FakeDriver, ImageInput, SessionDriver, SessionSpec, UserInput};
use serde_json::json;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/plain_turn.ndjson"
);

#[tokio::test]
async fn the_user_message_carries_image_blocks_before_the_text() {
    let fake = FakeDriver::from_file(FIXTURE).expect("fixture loads");
    let session = fake
        .start(SessionSpec::new("s-1", "/tmp"))
        .expect("session starts");
    let input = UserInput {
        text: "what is this?".into(),
        images: vec![ImageInput {
            media_type: "image/png".into(),
            data: b"foobar".to_vec(),
        }],
    };
    session.send(input).expect("send");
    let lines = fake.user_messages();
    assert_eq!(
        lines,
        vec![json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": "Zm9vYmFy"
                }},
                {"type": "text", "text": "what is this?"},
            ]},
            "parent_tool_use_id": null,
        })]
    );
}

#[test]
fn text_only_stays_a_plain_string_and_image_only_has_no_text_block() {
    let plain = UserInput::text("hi").to_stream_json();
    assert_eq!(plain["message"]["content"], json!("hi"));

    let image_only = UserInput {
        text: String::new(),
        images: vec![ImageInput {
            media_type: "image/gif".into(),
            data: b"f".to_vec(),
        }],
    }
    .to_stream_json();
    let blocks = image_only["message"]["content"]
        .as_array()
        .expect("content is an array");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["source"]["data"], json!("Zg=="));
}
