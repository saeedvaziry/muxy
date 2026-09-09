use std::collections::BTreeMap;

use muxy_protocol::ScreenFrame;

pub(super) fn merge(older: &mut ScreenFrame, newer: ScreenFrame) {
    if newer.reset {
        *older = newer;
        return;
    }
    let mut rows: BTreeMap<_, _> = older.rows.drain(..).map(|row| (row.index, row)).collect();
    rows.extend(newer.rows.into_iter().map(|row| (row.index, row)));
    older.rows = rows.into_values().collect();
    older.seq = newer.seq;
    older.cursor = newer.cursor;
    older.modes = newer.modes;
}

#[cfg(test)]
mod tests {
    use muxy_protocol::{Cursor, Modes, Row};

    use super::*;

    pub(super) fn frame(seq: u64, reset: bool, indexes: &[u16]) -> ScreenFrame {
        ScreenFrame {
            seq,
            reset,
            rows: indexes
                .iter()
                .map(|&index| Row {
                    index,
                    runs: vec![],
                })
                .collect(),
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: true,
            },
            modes: Modes {
                application_cursor_keys: false,
                bracketed_paste: false,
            },
        }
    }

    #[test]
    fn newer_rows_cursor_modes_and_sequence_win() {
        let mut older = frame(1, false, &[0, 1]);
        older.rows[1].runs.push(muxy_protocol::Run {
            text: "old".into(),
            width: 3,
            style: muxy_protocol::Style::default(),
        });
        let mut newer = frame(3, false, &[1, 2]);
        newer.cursor.col = 4;
        newer.modes.bracketed_paste = true;
        let expected = ScreenFrame {
            rows: frame(3, false, &[0, 1, 2]).rows,
            ..newer.clone()
        };
        merge(&mut older, newer);
        assert_eq!(older, expected);
    }

    #[test]
    fn newer_reset_drops_old_rows_and_survives_later_deltas() {
        let mut older = frame(1, false, &[0, 5]);
        let reset = frame(2, true, &[0, 1]);
        merge(&mut older, reset.clone());
        assert_eq!(older, reset);
        merge(&mut older, frame(4, false, &[1]));
        assert_eq!(older, frame(4, true, &[0, 1]));
    }
}
