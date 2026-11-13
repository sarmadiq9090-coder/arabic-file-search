//! Fitting the main window to the screen, and the result list's columns to
//! the window. Platform-independent and unit-tested on Linux; the Win32 side
//! is `ui/placement.rs` (the window) and `ui/build.rs` (the columns).

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

/// Column widths that fill `available` pixels of list, from each column's
/// (default width, least width). A column whose least width is its default
/// keeps it (sizes and dates need what they need); the others share the
/// rest in proportion to their defaults, none under its least width. When
/// even the least widths don't fit, they are returned and the list scrolls.
///
/// The defaults add up to 1310 pixels, wider than the list of the default
/// 1100-pixel window, so the last column's header was cut ("Mat…").
pub fn fit_columns(columns: &[(i32, i32)], available: i32) -> Vec<i32> {
    let mut out: Vec<i32> = columns.iter().map(|c| c.0).collect();
    let mut flexible: Vec<usize> = (0..columns.len()).filter(|&i| columns[i].1 < columns[i].0).collect();
    loop {
        let rigid: i64 = (0..columns.len()).filter(|i| !flexible.contains(i)).map(|i| out[i] as i64).sum();
        let room = available as i64 - rigid;
        let total: i64 = flexible.iter().map(|&i| columns[i].0 as i64).sum();
        if total <= 0 {
            return out;
        }
        // Columns the proportional share would make too narrow keep their
        // least width, and the others share what is left.
        let tight: Vec<usize> = flexible.iter().copied().filter(|&i| columns[i].0 as i64 * room < columns[i].1 as i64 * total).collect();
        if tight.is_empty() {
            let mut used = 0;
            for (k, &i) in flexible.iter().enumerate() {
                // The last one takes the rounding remainder.
                let w = if k + 1 == flexible.len() { room - used } else { columns[i].0 as i64 * room / total };
                out[i] = w.clamp(0, i32::MAX as i64) as i32;
                used += w;
            }
            return out;
        }
        for i in tight {
            out[i] = columns[i].1;
            flexible.retain(|&j| j != i);
        }
    }
}

/// Whether the saved column widths are not the user's own: none saved yet
/// (a first run), or exactly the defaults (saved by a version that had
/// only those), so the columns may be fitted to the window.
pub fn columns_untouched(saved: &[i32], columns: &[(i32, i32)]) -> bool {
    saved.is_empty() || saved.iter().copied().eq(columns.iter().map(|c| c.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Name, full path, size, date modified, matching text (`ui::COLUMNS`).
    const COLUMNS: [(i32, i32); 5] = [(260, 160), (460, 200), (90, 90), (140, 140), (360, 160)];

    #[test]
    fn default_columns_fit_the_default_window() {
        // The list of the 1100-pixel window, without its scroll bar.
        let w = fit_columns(&COLUMNS, 1054);
        assert_eq!(w.iter().sum::<i32>(), 1054);
        assert_eq!(w, [198, 350, 90, 140, 276]);
        // "Matching text" keeps room for its header and a snippet.
        assert!(w[4] >= 160, "{w:?}");
    }

    #[test]
    fn columns_fill_any_list_width_and_keep_their_least_widths() {
        let least: i32 = COLUMNS.iter().map(|c| c.1).sum();
        for available in (300..4000).step_by(7) {
            let w = fit_columns(&COLUMNS, available);
            // Size and date never change; no column goes under its least width.
            assert_eq!((w[2], w[3]), (90, 140), "{available}");
            for (got, (_, min)) in w.iter().zip(COLUMNS) {
                assert!(*got >= min, "{available}: {w:?}");
            }
            // The list is filled exactly whenever the least widths fit.
            assert_eq!(w.iter().sum::<i32>(), available.max(least), "{available}: {w:?}");
        }
    }

    #[test]
    fn narrow_and_wide_lists() {
        // 800-pixel window: the name and the snippet are at their least
        // widths, and the path takes the rest.
        assert_eq!(fit_columns(&COLUMNS, 754), [160, 204, 90, 140, 160]);
        // Too narrow for everything: least widths, and the list scrolls.
        assert_eq!(fit_columns(&COLUMNS, 400), [160, 200, 90, 140, 160]);
        assert_eq!(fit_columns(&COLUMNS, -50), [160, 200, 90, 140, 160]);
        // A maximized window on a 1920-pixel screen: the text columns grow.
        let wide = fit_columns(&COLUMNS, 1874);
        assert_eq!(wide.iter().sum::<i32>(), 1874);
        assert!(wide[0] > 260 && wide[1] > 460 && wide[4] > 360, "{wide:?}");
        // The proportions of the defaults are kept (260 : 460 : 360).
        assert!((wide[1] * 260 - wide[0] * 460).abs() < 460 * 2, "{wide:?}");
        // Nothing to share, or nothing at all.
        assert_eq!(fit_columns(&[(90, 90), (140, 140)], 1000), [90, 140]);
        assert_eq!(fit_columns(&[], 1000), Vec::<i32>::new());
    }

    #[test]
    fn only_widths_the_user_never_set_are_fitted() {
        assert!(columns_untouched(&[], &COLUMNS));
        // Saved by 0.10.0 and earlier on every close: the old defaults.
        assert!(columns_untouched(&[260, 460, 90, 140, 360], &COLUMNS));
        // The user's own widths (or ones fitted before) stay.
        assert!(!columns_untouched(&[300, 460, 90, 140, 360], &COLUMNS));
        assert!(!columns_untouched(&[198, 350, 90, 140, 276], &COLUMNS));
        assert!(!columns_untouched(&[260, 460, 90, 140], &COLUMNS));
    }

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
