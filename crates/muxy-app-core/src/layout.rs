use serde::{Deserialize, Serialize};

use crate::{AppError, PaneId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Branch {
    First,
    Second,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Layout {
    Leaf(PaneId),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut leaves = Vec::new();
        self.visit(&mut |pane, _| leaves.push(pane), [0.0, 0.0, 1.0, 1.0]);
        leaves
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Leaf(id) => *id == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    pub(crate) fn split(&mut self, pane: PaneId, new: PaneId, edge: Direction) {
        match self {
            Self::Leaf(id) if *id == pane => {
                let (first, second) = if matches!(edge, Direction::Left | Direction::Up) {
                    (new, pane)
                } else {
                    (pane, new)
                };
                *self = Self::Split {
                    axis: edge.axis(),
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(first)),
                    second: Box::new(Self::Leaf(second)),
                };
            }
            Self::Split { first, second, .. } => {
                first.split(pane, new, edge);
                second.split(pane, new, edge);
            }
            Self::Leaf(_) => {}
        }
    }

    pub(crate) fn remove(&mut self, pane: PaneId) {
        if let Self::Split { first, second, .. } = self {
            if matches!(first.as_ref(), Self::Leaf(id) if *id == pane) {
                *self = *second.clone();
            } else if matches!(second.as_ref(), Self::Leaf(id) if *id == pane) {
                *self = *first.clone();
            } else {
                first.remove(pane);
                second.remove(pane);
            }
        }
    }

    pub fn set_ratio(&mut self, path: &[Branch], value: f32) -> Result<(), AppError> {
        if !value.is_finite() {
            return Err(AppError::InvalidState("split ratio must be finite".into()));
        }
        let mut node = self;
        for branch in path {
            let Self::Split { first, second, .. } = node else {
                return Err(AppError::InvalidState("unknown split path".into()));
            };
            node = match branch {
                Branch::First => first,
                Branch::Second => second,
            };
        }
        let Self::Split { ratio, .. } = node else {
            return Err(AppError::InvalidState("path must name a split".into()));
        };
        *ratio = value.clamp(0.15, 0.85);
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            if !ratio.is_finite() || !(0.15..=0.85).contains(ratio) {
                return Err(AppError::InvalidState(
                    "split ratio must be between 0.15 and 0.85".into(),
                ));
            }
            first.validate()?;
            second.validate()?;
        }
        Ok(())
    }

    pub fn neighbor(&self, pane: PaneId, direction: Direction) -> Option<PaneId> {
        let mut rectangles = Vec::new();
        self.visit(
            &mut |id, rect| rectangles.push((id, rect)),
            [0.0, 0.0, 1.0, 1.0],
        );
        let (_, source) = rectangles.iter().find(|(id, _)| *id == pane)?;
        let [x, y, width, height] = *source;
        rectangles
            .iter()
            .filter_map(|(id, rect)| {
                if *id == pane {
                    return None;
                }
                let [other_x, other_y, other_width, other_height] = *rect;
                let (gap, overlap, offset) = match direction {
                    Direction::Left | Direction::Right => (
                        if direction == Direction::Left {
                            x - other_x - other_width
                        } else {
                            other_x - x - width
                        },
                        (y + height).min(other_y + other_height) - y.max(other_y),
                        (y + height / 2.0 - other_y - other_height / 2.0).abs(),
                    ),
                    Direction::Up | Direction::Down => (
                        if direction == Direction::Up {
                            y - other_y - other_height
                        } else {
                            other_y - y - height
                        },
                        (x + width).min(other_x + other_width) - x.max(other_x),
                        (x + width / 2.0 - other_x - other_width / 2.0).abs(),
                    ),
                };
                (gap.abs() < 0.000_01 && overlap > 0.0).then_some((*id, offset))
            })
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(id, _)| id)
    }

    fn visit(&self, visitor: &mut impl FnMut(PaneId, [f32; 4]), rect: [f32; 4]) {
        match self {
            Self::Leaf(id) => visitor(*id, rect),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let mut first_rect = rect;
                let mut second_rect = rect;
                let dimension = match axis {
                    Axis::Horizontal => 0,
                    Axis::Vertical => 1,
                };
                first_rect[dimension + 2] *= ratio;
                second_rect[dimension] += first_rect[dimension + 2];
                second_rect[dimension + 2] *= 1.0 - ratio;
                first.visit(visitor, first_rect);
                second.visit(visitor, second_rect);
            }
        }
    }
}
