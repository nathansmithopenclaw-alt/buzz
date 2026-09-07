use buzz_sdk::{
    build_edit, build_forum_comment, build_forum_post, build_message, SdkError, ThreadRef,
};
use nostr::{EventBuilder, EventId, Keys};
use uuid::Uuid;

fn messages(content: &str, media: &[Vec<String>]) -> [Result<EventBuilder, SdkError>; 3] {
    let channel = Uuid::new_v4();
    let parent = EventId::all_zeros();
    let thread = ThreadRef {
        root_event_id: parent,
        parent_event_id: parent,
    };
    [
        build_message(channel, content, None, &[], false, media, &[]),
        build_forum_post(channel, content, &[], media),
        build_forum_comment(channel, content, &thread, &[], media),
    ]
}

#[test]
fn new_messages_reject_empty_and_unicode_whitespace() {
    for content in ["", " \t\r\n", "\u{00a0}\u{2003}\u{2028}\u{3000}"] {
        for result in messages(content, &[]) {
            assert!(matches!(result, Err(SdkError::InvalidInput(message))
                if message.contains("empty or whitespace-only")));
        }
    }
}

#[test]
fn unrelated_or_unusable_tags_do_not_replace_message_content() {
    for tag in [
        vec![],
        vec!["imeta"],
        vec!["imeta", "url "],
        vec!["imeta", "url not-a-url"],
        vec!["imeta", "url file:///tmp/photo.png"],
        vec!["p", "url https://example.com/photo.png"],
    ] {
        let media = vec![tag.into_iter().map(str::to_owned).collect()];
        for result in messages("", &media) {
            assert!(matches!(result, Err(SdkError::InvalidInput(_))));
        }
    }
}

#[test]
fn new_messages_preserve_attachment_only_payloads() {
    let media = vec![vec![
        "imeta".into(),
        "url https://example.com/photo.png".into(),
        "m image/png".into(),
    ]];
    let keys = Keys::generate();
    for builder in messages("", &media) {
        let event = builder
            .expect("attachment payload")
            .sign_with_keys(&keys)
            .expect("sign");
        assert!(event.content.is_empty());
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == media[0].as_slice()));
    }
}

#[test]
fn new_messages_preserve_exact_markdown_and_unicode_text() {
    let keys = Keys::generate();
    for content in [
        "  indented  \n\tMarkdown `code` $literal\n",
        "🐝",
        "\u{200b}",
    ] {
        for builder in messages(content, &[]) {
            let event = builder
                .expect("text payload")
                .sign_with_keys(&keys)
                .expect("sign");
            assert_eq!(event.content, content);
        }
    }
}

#[test]
fn clearing_a_message_edit_remains_valid() {
    let event = build_edit(Uuid::new_v4(), EventId::all_zeros(), "")
        .expect("empty edit")
        .sign_with_keys(&Keys::generate())
        .expect("sign");
    assert_eq!(event.kind.as_u16(), 40003);
    assert!(event.content.is_empty());
}
