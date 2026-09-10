use muxy_protocol::{
    ErrorCode, LinkRow, LinkSpan, MAX_LINK_SPANS, MAX_LINK_URI, Message, MetadataEvent,
};

fn row() -> LinkRow {
    LinkRow {
        row: 0,
        spans: vec![LinkSpan {
            start: 0,
            end: 4,
            uri: "https://example.com".into(),
        }],
    }
}

#[test]
fn hyperlink_metadata_is_a_bounded_ordered_whole_screen_replacement() {
    let validate = |rows| Message::Metadata(MetadataEvent::Links { seq: 1, rows }).validate();
    assert_eq!(validate(vec![row()]), Ok(()));
    assert_eq!(validate(vec![]), Ok(()));
    let mut invalid = vec![vec![row(), row()]];
    for mutate in [
        |row: &mut LinkRow| row.row = 1024,
        |row: &mut LinkRow| row.spans[0].end = 0,
        |row: &mut LinkRow| row.spans[0].end = 4097,
        |row: &mut LinkRow| row.spans[0].uri.clear(),
        |row: &mut LinkRow| row.spans[0].uri = "a".repeat(MAX_LINK_URI + 1),
        |row: &mut LinkRow| row.spans[0].uri.push('\0'),
        |row: &mut LinkRow| row.spans = vec![row.spans[0].clone(); MAX_LINK_SPANS + 1],
        |row: &mut LinkRow| row.spans.push(row.spans[0].clone()),
    ] {
        let mut value = row();
        mutate(&mut value);
        invalid.push(vec![value]);
    }
    for rows in invalid {
        assert_eq!(validate(rows), Err(ErrorCode::BadRequest));
    }
}
