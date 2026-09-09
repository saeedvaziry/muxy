use muxy_protocol::{
    Cursor, ErrorCode, ExitReason, Message, ReplyBody, RequestId, Row, Run, SavedScreen, Size,
    Style,
};

fn screen() -> SavedScreen {
    SavedScreen {
        size: Size { cols: 4, rows: 1 },
        rows: vec![Row {
            index: 0,
            runs: vec![Run {
                text: "done".into(),
                width: 4,
                style: Style::default(),
            }],
        }],
        cursor: Cursor {
            row: 0,
            col: 3,
            visible: false,
        },
        reason: Some(ExitReason::Exited(0)),
    }
}

fn validate(screen: SavedScreen) -> Result<(), ErrorCode> {
    Message::Reply {
        id: RequestId(1),
        body: ReplyBody::SavedScreen(screen),
    }
    .validate()
}

#[test]
fn saved_screens_require_a_complete_grid_and_valid_cell_coordinates() {
    assert_eq!(validate(screen()), Ok(()));
    let mut invalid = screen();
    invalid.rows.clear();
    assert_eq!(validate(invalid), Err(ErrorCode::BadRequest));
    let mut invalid = screen();
    invalid.rows[0].index = 1;
    assert_eq!(validate(invalid), Err(ErrorCode::BadRequest));
    let mut invalid = screen();
    invalid.rows[0].runs[0].width = 5;
    assert_eq!(validate(invalid), Err(ErrorCode::BadRequest));
    let mut invalid = screen();
    invalid.rows[0].runs[0].width = 0;
    assert_eq!(validate(invalid), Err(ErrorCode::BadRequest));
    let mut invalid = screen();
    invalid.cursor.col = 4;
    assert_eq!(validate(invalid), Err(ErrorCode::BadRequest));
    let mut invalid = screen();
    invalid.size.rows = 0;
    assert_eq!(validate(invalid), Err(ErrorCode::BadSize));
}
