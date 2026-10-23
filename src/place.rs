//! Where a new clock or the settings window should open.
//!
//! Coordinates are outer top-left corners in the same space as `work`
//! (egui points, origin at the top-left of the virtual screen).

/// A rectangle: outer position plus size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    fn max_x(self) -> f32 {
        self.x + self.width
    }

    fn max_y(self) -> f32 {
        self.y + self.height
    }
}

const CASCADE_STEP: f32 = 32.0;
const BESIDE_GAP: f32 = 12.0;

/// Outer top-left of a clock opened from another clock.
///
/// `step` is 1 for the first clock that window opens. The new window sits
/// that many steps above and to the right, so the one underneath still shows.
/// When the step would leave the work area, the same distance goes downward.
pub fn cascade_above(parent: (f32, f32), child: (f32, f32), work: Bounds, step: u32) -> (f32, f32) {
    let distance = CASCADE_STEP * step.max(1) as f32;
    let x = parent.0 + distance;
    let above = parent.1 - distance;
    let y = if above >= work.y {
        above
    } else {
        parent.1 + distance
    };
    clamp_inside((x, y), child, work)
}

/// Outer top-left of a window placed beside `anchor`, preferring the right side.
pub fn beside(anchor: Bounds, window: (f32, f32), work: Bounds) -> (f32, f32) {
    let right = anchor.max_x() + BESIDE_GAP;
    let left = anchor.x - BESIDE_GAP - window.0;
    let x = if right + window.0 <= work.max_x() {
        right
    } else if left >= work.x {
        left
    } else {
        work.x
    };
    clamp_inside((x, anchor.y), window, work)
}

/// Outer top-left that centers `window` in `work`.
pub fn center(work: Bounds, window: (f32, f32)) -> (f32, f32) {
    let origin = (
        work.x + (work.width - window.0) * 0.5,
        work.y + (work.height - window.1) * 0.5,
    );
    clamp_inside(origin, window, work)
}

fn clamp_inside(origin: (f32, f32), window: (f32, f32), work: Bounds) -> (f32, f32) {
    let max_x = (work.max_x() - window.0).max(work.x);
    let max_y = (work.max_y() - window.1).max(work.y);
    (origin.0.clamp(work.x, max_x), origin.1.clamp(work.y, max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Bounds {
        Bounds {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }
    }

    #[test]
    fn cascade_steps_up_and_to_the_right() {
        let parent = (100.0, 200.0);
        let child = (300.0, 220.0);
        assert_eq!(cascade_above(parent, child, work(), 1), (132.0, 168.0));
        assert_eq!(cascade_above(parent, child, work(), 2), (164.0, 136.0));
    }

    #[test]
    fn cascade_drops_down_when_the_top_is_too_close() {
        let child = (300.0, 220.0);
        let origin = cascade_above((80.0, 20.0), child, work(), 1);
        assert_eq!(origin, (112.0, 52.0));
    }

    #[test]
    fn cascade_stays_inside_the_work_area() {
        let child = (300.0, 220.0);
        let origin = cascade_above((1800.0, 200.0), child, work(), 4);
        assert!(origin.0 + child.0 <= 1920.0);
        assert!(origin.1 >= 0.0);
    }

    #[test]
    fn settings_open_on_the_right_when_there_is_room() {
        let clock = Bounds {
            x: 100.0,
            y: 80.0,
            width: 300.0,
            height: 220.0,
        };
        assert_eq!(beside(clock, (420.0, 460.0), work()), (412.0, 80.0));
    }

    #[test]
    fn settings_open_on_the_left_when_the_right_is_blocked() {
        let clock = Bounds {
            x: 1500.0,
            y: 80.0,
            width: 300.0,
            height: 220.0,
        };
        let origin = beside(clock, (420.0, 460.0), work());
        assert_eq!(origin.0, 1500.0 - 12.0 - 420.0);
        assert_eq!(origin.1, 80.0);
    }

    #[test]
    fn a_window_near_the_bottom_is_lifted_into_the_work_area() {
        let clock = Bounds {
            x: 100.0,
            y: 900.0,
            width: 300.0,
            height: 220.0,
        };
        let origin = beside(clock, (420.0, 460.0), work());
        assert_eq!(origin.1, 1080.0 - 460.0);
    }

    #[test]
    fn center_uses_the_middle_of_the_work_area() {
        let area = Bounds {
            x: 1920.0,
            y: 40.0,
            width: 1600.0,
            height: 1000.0,
        };
        assert_eq!(center(area, (300.0, 220.0)), (2570.0, 430.0));
    }
}
