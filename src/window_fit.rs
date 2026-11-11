//! Fitting the main window to the screen. Platform-independent and
//! unit-tested on Linux; the Win32 side is `ui/placement.rs`.

// Only the Windows UI and the tests use this module.

use crate::quick::Rect;

/// Where a window of `size` (outer width and height) goes on a monitor
/// whose usable area (without the taskbar) is `work`: at `at` (its top-left
/// corner) when it has a place already, centred in the work area otherwise.
/// Either way the result lies fully inside the work area: a window larger
/// than it is cut to its size, and one that sticks out (a place kept from
/// another screen, a taskbar that moved or grew) is moved back in.
///
/// Windows centres on the whole monitor, so on a 1366×768 screen the
/// window's last 20 pixels (the status line) went under the taskbar.
pub fn place(work: &Rect, size: (i32, i32), at: Option<(i32, i32)>) -> Rect {
    let (w, h) = (size.0.max(0), size.1.max(0));
    if work.is_empty() {
        // Nothing is known about the screen: leave the window as it is.
        let (x, y) = at.unwrap_or((work.left, work.top));
        return Rect::at(x, y, w, h);
    }
    let (w, h) = (w.min(work.width()), h.min(work.height()));
    let (x, y) = at.unwrap_or((work.left + (work.width() - w) / 2, work.top + (work.height() - h) / 2));
    Rect::at(x.clamp(work.left, work.right - w), y.clamp(work.top, work.bottom - h), w, h)
}

/// The area a resizable window's rectangle may fill so that the frame one
/// sees stays inside `work`: Windows 10 and 11 keep `frame` pixels of the
/// resize frame invisible on the left, right and bottom (a window snapped to
/// half the screen has a rectangle that much larger than the half).
pub fn with_hidden_frame(work: &Rect, frame: i32) -> Rect {
    let f = frame.clamp(0, 64);
    if work.is_empty() {
        return *work;
    }
    Rect::new(work.left.saturating_sub(f), work.top, work.right.saturating_add(f), work.bottom.saturating_add(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default window (1100×680 of content) with its frame at 100 %.
    const OUTER: (i32, i32) = (1116, 719);

    fn inside(r: &Rect, work: &Rect) -> bool {
        r.left >= work.left && r.top >= work.top && r.right <= work.right && r.bottom <= work.bottom
    }

    #[test]
    fn a_768_pixel_screen_keeps_the_status_line_above_the_taskbar() {
        // 1366×768 with a 40-pixel taskbar at the bottom. Centred on the
        // whole screen the window ended at y = 743, under the taskbar (728).
        let work = Rect::new(0, 0, 1366, 728);
        let r = place(&work, OUTER, None);
        assert_eq!((r.width(), r.height()), OUTER);
        assert_eq!(r, Rect::at(125, 4, 1116, 719));
        assert!(r.bottom <= 728, "{r:?}");
    }

    #[test]
    fn centred_in_the_work_area_for_a_taskbar_on_each_edge() {
        let monitor = Rect::new(0, 0, 1920, 1080);
        let size = (1000, 700);
        for work in [
            Rect::new(0, 0, 1920, 1032),  // bottom
            Rect::new(0, 48, 1920, 1080), // top
            Rect::new(72, 0, 1920, 1080), // left
            Rect::new(0, 0, 1848, 1080),  // right
        ] {
            let r = place(&work, size, None);
            assert!(inside(&r, &work) && inside(&r, &monitor), "{work:?} -> {r:?}");
            assert_eq!((r.width(), r.height()), size);
            // The same room on both sides, within a pixel.
            assert!(((r.left - work.left) - (work.right - r.right)).abs() <= 1, "{work:?} -> {r:?}");
            assert!(((r.top - work.top) - (work.bottom - r.bottom)).abs() <= 1, "{work:?} -> {r:?}");
        }
    }

    #[test]
    fn a_window_larger_than_the_work_area_is_cut_to_it() {
        // 1080p at 150 %: the default window needs 1674×1079 pixels, the
        // work area is 1920×1020; and a 1024×600 netbook at 100 %.
        for (work, size) in [(Rect::new(0, 0, 1920, 1020), (1674, 1079)), (Rect::new(0, 0, 1024, 560), OUTER)] {
            let r = place(&work, size, None);
            assert!(inside(&r, &work), "{work:?} -> {r:?}");
            assert_eq!(r.width(), size.0.min(work.width()));
            assert_eq!(r.height(), work.height());
            assert_eq!(r.top, work.top);
        }
    }

    #[test]
    fn monitors_with_negative_coordinates() {
        // A second monitor to the left of, and one above, the primary.
        for work in [Rect::new(-1920, 0, 0, 1040), Rect::new(-1280, -1024, 0, -40), Rect::new(-1366, 200, 0, 928)] {
            let r = place(&work, OUTER, None);
            assert!(inside(&r, &work), "{work:?} -> {r:?}");
            assert_eq!((r.width(), r.height()), (OUTER.0.min(work.width()), OUTER.1.min(work.height())));
            assert!(((r.left - work.left) - (work.right - r.right)).abs() <= 1, "{work:?} -> {r:?}");
        }
        let r = place(&Rect::new(-1920, 0, 0, 1040), OUTER, None);
        assert_eq!(r, Rect::at(-1518, 160, 1116, 719));
    }

    #[test]
    fn a_place_that_is_still_on_screen_is_kept() {
        let work = Rect::new(0, 0, 1920, 1040);
        assert_eq!(place(&work, (900, 600), Some((300, 200))), Rect::at(300, 200, 900, 600));
        // Touching the edges is fine too.
        assert_eq!(place(&work, (900, 600), Some((1020, 440))), Rect::at(1020, 440, 900, 600));
        assert_eq!(place(&Rect::new(-1920, 0, 0, 1040), (900, 600), Some((-1900, 10))), Rect::at(-1900, 10, 900, 600));
    }

    #[test]
    fn a_place_from_another_monitor_setup_is_pulled_back() {
        let laptop = Rect::new(0, 0, 1366, 728);
        // It was on a monitor to the right that is gone now.
        let r = place(&laptop, OUTER, Some((2200, 150)));
        assert!(inside(&r, &laptop), "{r:?}");
        assert_eq!(r, Rect::at(250, 9, 1116, 719));
        // It was on a monitor to the left, or above; or its bottom is under
        // the taskbar; or it was sized for a bigger screen.
        for (size, at) in [(OUTER, (-1800, 100)), (OUTER, (100, -900)), (OUTER, (100, 60)), ((1800, 1000), (40, 20)), ((900, 600), (i32::MAX - 10, i32::MIN + 10))] {
            let r = place(&laptop, size, Some(at));
            assert!(inside(&r, &laptop), "{size:?} at {at:?} -> {r:?}");
            assert_eq!((r.width(), r.height()), (size.0.min(1366), size.1.min(728)));
        }
        // Only as far as needed: a window whose bottom was 51 pixels under
        // the taskbar moves up by 51 and stays in its column.
        assert_eq!(place(&laptop, OUTER, Some((100, 60))), Rect::at(100, 9, 1116, 719));
    }

    #[test]
    fn a_snapped_window_is_not_nudged() {
        // Snapped to the left half of a 1920×1040 work area, Windows gives
        // the window 7 invisible frame pixels on the left, right and bottom.
        let work = Rect::new(0, 0, 1920, 1040);
        let area = with_hidden_frame(&work, 7);
        assert_eq!(area, Rect::new(-7, 0, 1927, 1047));
        let snapped = Rect::new(-7, 0, 967, 1047);
        assert_eq!(place(&area, (snapped.width(), snapped.height()), Some((snapped.left, snapped.top))), snapped);
        // The title bar never goes above the work area, and what one sees
        // of a window placed in that area is inside the work area.
        let r = place(&area, OUTER, Some((1500, 900)));
        assert_eq!(r, Rect::at(811, 328, 1116, 719));
        assert!(r.top >= work.top && r.right - 7 <= work.right && r.bottom - 7 <= work.bottom && r.left + 7 >= work.left);
        assert_eq!(with_hidden_frame(&Rect::default(), 7), Rect::default());
        assert_eq!(with_hidden_frame(&work, -3), work);
    }

    #[test]
    fn an_unknown_screen_leaves_the_window_alone() {
        let nothing = Rect::default();
        assert_eq!(place(&nothing, (900, 600), Some((300, 200))), Rect::at(300, 200, 900, 600));
        assert_eq!(place(&nothing, (900, 600), None), Rect::at(0, 0, 900, 600));
        assert!(place(&Rect::new(0, 0, 800, 600), (-5, -5), None).is_empty());
    }
}
