use muxy_protocol::{
    ChannelId, ErrorCode, HistoryCursor, Message, ReplyBody, RequestBody, RequestId, SearchMatch,
    SearchPage, SearchSource,
};

fn request(query: String, max_results: u16) -> Message {
    Message::Request {
        id: RequestId(1),
        body: RequestBody::Search {
            source: SearchSource::Live(ChannelId(1)),
            query,
            ignore_case: true,
            before: HistoryCursor(0),
            max_results,
        },
    }
}

#[test]
fn queries_are_bounded_by_utf8_bytes_and_result_count() {
    for (query, results, valid) in [
        (String::new(), 1, false),
        ("a".repeat(256), 500, true),
        ("é".repeat(128), 1, true),
        ("é".repeat(129), 1, false),
        ("a".into(), 0, false),
        ("a".into(), 501, false),
    ] {
        assert_eq!(request(query, results).validate().is_ok(), valid);
    }
}

#[test]
fn empty_pages_may_continue_but_malformed_results_are_rejected() {
    let mut page = SearchPage {
        matches: Vec::new(),
        next: Some(HistoryCursor(42)),
        total_rows: 5000,
        scanned_rows: 2000,
    };
    let valid = |page| {
        Message::Reply {
            id: RequestId(1),
            body: ReplyBody::SearchPage(page),
        }
        .validate()
    };
    assert!(valid(page.clone()).is_ok());
    page.scanned_rows = 2001;
    assert_eq!(valid(page.clone()), Err(ErrorCode::BadRequest));
    page.scanned_rows = 1;
    page.matches = vec![SearchMatch {
        row: 10,
        start: 4,
        end: 4,
    }];
    assert!(valid(page.clone()).is_err());
    page.matches[0].end = 5;
    assert!(valid(page.clone()).is_ok());
    page.matches.push(page.matches[0]);
    assert!(valid(page).is_err());
}
