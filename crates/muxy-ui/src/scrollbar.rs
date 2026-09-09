use std::time::Duration;

pub const REVEAL_DURATION: Duration = Duration::from_millis(1_250);
pub const TRACK_INSET: f32 = 8.0;
pub const WIDTH: f32 = 7.0;
pub const MINIMUM_THUMB_LENGTH: f64 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThumbGeometry {
    pub origin: f64,
    pub length: f64,
}

impl ThumbGeometry {
    pub fn from_lengths(
        total: f64,
        visible: f64,
        offset: f64,
        track_length: f64,
        minimum_thumb_length: f64,
    ) -> Option<Self> {
        if !total.is_finite()
            || !visible.is_finite()
            || !offset.is_finite()
            || total <= 0.0
            || visible <= 0.0
            || visible >= total
        {
            return None;
        }

        let maximum_offset = total - visible;
        Self::from_proportions(
            visible / total,
            offset.clamp(0.0, maximum_offset) / maximum_offset,
            track_length,
            minimum_thumb_length,
        )
    }

    fn from_proportions(
        visible_fraction: f64,
        offset_fraction: f64,
        track_length: f64,
        minimum_thumb_length: f64,
    ) -> Option<Self> {
        if !track_length.is_finite() || track_length <= 0.0 {
            return None;
        }
        let minimum = if minimum_thumb_length.is_finite() {
            minimum_thumb_length.clamp(0.0, track_length)
        } else {
            0.0
        };
        let length = (track_length * visible_fraction).clamp(minimum, track_length);
        let travel = (track_length - length).max(0.0);
        let origin = travel * offset_fraction;

        Some(Self {
            origin: origin.clamp(0.0, travel),
            length,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScrollbarRevealState {
    revealed_until: Option<Duration>,
    dragging: bool,
}

impl ScrollbarRevealState {
    pub fn reveal(&mut self, now: Duration) {
        self.revealed_until = now.checked_add(REVEAL_DURATION).or(Some(Duration::MAX));
    }

    pub fn begin_drag(&mut self) {
        self.dragging = true;
    }

    pub fn end_drag(&mut self, now: Duration) {
        self.dragging = false;
        self.reveal(now);
    }

    pub fn allows_hit(self, now: Duration) -> bool {
        self.dragging || self.revealed_until.is_some_and(|until| now < until)
    }

    pub fn extend_near_track(
        &mut self,
        now: Duration,
        pointer_x: f64,
        view_width: f64,
        scroller_width: f64,
    ) -> bool {
        if !pointer_x.is_finite()
            || !view_width.is_finite()
            || !scroller_width.is_finite()
            || pointer_x < view_width - scroller_width.max(0.0) * 2.0
        {
            return false;
        }

        self.reveal(now);
        true
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "These geometry cases use exactly representable values."
)]
mod tests {
    use super::*;

    #[test]
    fn length_geometry_rejects_invalid_and_non_scrollable_values() {
        assert_eq!(
            ThumbGeometry::from_lengths(100.0, 10.0, 0.0, 0.0, 24.0),
            None
        );
        assert_eq!(
            ThumbGeometry::from_lengths(100.0, 100.0, 0.0, 100.0, 24.0),
            None
        );
        assert_eq!(
            ThumbGeometry::from_lengths(f64::NAN, 10.0, 0.0, 100.0, 24.0),
            None
        );
        assert_eq!(
            ThumbGeometry::from_lengths(100.0, 10.0, f64::INFINITY, 100.0, 24.0),
            None
        );
    }

    #[test]
    fn length_geometry_tracks_offset_and_respects_minimum_thumb()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            ThumbGeometry::from_lengths(200.0, 50.0, 0.0, 100.0, 24.0),
            Some(ThumbGeometry {
                origin: 0.0,
                length: 25.0,
            })
        );
        assert_eq!(
            ThumbGeometry::from_lengths(200.0, 50.0, 150.0, 100.0, 24.0),
            Some(ThumbGeometry {
                origin: 75.0,
                length: 25.0,
            })
        );
        assert_eq!(
            ThumbGeometry::from_lengths(1000.0, 10.0, 0.0, 100.0, 24.0)
                .ok_or("missing geometry")?
                .length,
            24.0
        );
        Ok(())
    }

    #[test]
    fn reveal_is_serialized_with_drag_state() {
        let mut reveal = ScrollbarRevealState::default();
        let now = Duration::from_secs(10);
        assert!(!reveal.allows_hit(now));

        reveal.reveal(now);
        assert!(reveal.allows_hit(now + Duration::from_secs(1)));
        assert!(!reveal.allows_hit(now + REVEAL_DURATION));

        reveal.begin_drag();
        assert!(reveal.allows_hit(Duration::from_secs(100)));
        reveal.end_drag(Duration::from_secs(100));
        assert!(reveal.allows_hit(Duration::from_secs(101)));
    }

    #[test]
    fn proximity_reveals_hidden_thumb_and_ignores_distant_pointer() {
        let mut reveal = ScrollbarRevealState::default();
        let now = Duration::from_secs(1);
        assert!(reveal.extend_near_track(now, 99.0, 100.0, 8.0));

        assert!(!reveal.extend_near_track(now, 50.0, 100.0, 8.0));
        assert!(reveal.extend_near_track(now, 90.0, 100.0, 8.0));
        assert!(
            reveal.allows_hit(
                (now + REVEAL_DURATION)
                    .checked_sub(Duration::from_millis(1))
                    .unwrap()
            )
        );
    }
}
