use muxy_protocol::{ErrorCode, Message, MetadataEvent, ReplyBody};

#[test]
fn prompt_metadata_is_bounded_sorted_and_unique() {
    for rows in [vec![0, 3, 1023], vec![]] {
        assert!(
            Message::Metadata(MetadataEvent::ScreenPrompts { seq: 1, rows })
                .validate()
                .is_ok()
        );
    }
    for rows in [vec![1024], vec![0, 0], vec![2, 1]] {
        assert_eq!(
            Message::Metadata(MetadataEvent::ScreenPrompts { seq: 1, rows }).validate(),
            Err(ErrorCode::BadRequest)
        );
    }
}

#[test]
fn snapshots_and_pages_reject_prompt_indexes_outside_their_own_rows() {
    for mut message in Message::samples() {
        let Message::Reply { body, .. } = &mut message else {
            continue;
        };
        match body {
            ReplyBody::Attached { snapshot, .. } => snapshot.prompts = vec![u16::MAX],
            ReplyBody::HistoryPage(page) => page.prompts = vec![u16::MAX],
            _ => continue,
        }
        assert_eq!(message.validate(), Err(ErrorCode::BadRequest));
    }
}
