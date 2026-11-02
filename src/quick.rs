//! The platform-independent parts of the quick search (0.8): where the
//! floating box and the popup go, how the box fades, which results are shown
//! in which order, and the pixel work behind the translucent drawing.
//!
//! The Win32 windows live in `src/ui/quick.rs` (popup), `src/ui/floating.rs`
//! (the box above the clock) and `src/ui/tray.rs`; everything they can
//! compute without Windows is here, so it is unit-tested on Linux.

use crate::launcher::{Action, Item, Kind};
use crate::settings::{FloatPos, FloatingMode};

// ------------------------------------------------------------------ geometry

/// A rectangle in screen pixels; `right` and `bottom` are exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    /// A rectangle `w`×`h` with its top-left corner at (`x`, `y`).
    pub const fn at(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { left: x, top: y, right: x.saturating_add(w), bottom: y.saturating_add(h) }
    }

    pub fn width(&self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    pub fn height(&self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub fn center(&self) -> (i32, i32) {
        (self.left + self.width() / 2, self.top + self.height() / 2)
    }

    /// The overlap of two rectangles (empty when they don't overlap).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let r = Rect::new(self.left.max(o.left), self.top.max(o.top), self.right.min(o.right), self.bottom.min(o.bottom));
        if r.is_empty() { Rect::default() } else { r }
    }

    /// Moved (not resized) to lie inside `outer`; a rectangle larger than
    /// `outer` is cut to its size.
    pub fn clamp_into(&self, outer: &Rect) -> Rect {
        let w = self.width().min(outer.width()).max(0);
        let h = self.height().min(outer.height()).max(0);
        let x = self.left.clamp(outer.left, (outer.right - w).max(outer.left));
        let y = self.top.clamp(outer.top, (outer.bottom - h).max(outer.top));
        Rect::at(x, y, w, h)
    }

    /// Distance in pixels from (`x`, `y`) to the nearest point of the
    /// rectangle; 0 inside.
    pub fn distance(&self, x: i32, y: i32) -> i32 {
        let dx = (self.left - x).max(x - (self.right - 1)).max(0) as i64;
        let dy = (self.top - y).max(y - (self.bottom - 1)).max(0) as i64;
        ((dx * dx + dy * dy) as f64).sqrt().round().min(i32::MAX as f64) as i32
    }
}

/// `v` logical pixels at `scale` (1.0 = 96 DPI).
pub fn px(v: i32, scale: f64) -> i32 {
    (v as f64 * scale.clamp(0.5, 8.0)).round() as i32
}

/// The screen edge the taskbar sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

impl Edge {
    /// From `APPBARDATA::uEdge` (`ABE_LEFT` = 0 … `ABE_BOTTOM` = 3).
    pub fn from_abe(edge: u32) -> Edge {
        match edge {
            0 => Edge::Left,
            1 => Edge::Top,
            2 => Edge::Right,
            _ => Edge::Bottom,
        }
    }

    pub fn horizontal(self) -> bool {
        matches!(self, Edge::Top | Edge::Bottom)
    }
}

/// What Windows says about the taskbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taskbar {
    /// `ABM_GETTASKBARPOS`; for an auto-hide taskbar this may be where it
    /// hides, see [`Taskbar::shown_rect`].
    pub rect: Rect,
    pub edge: Edge,
    /// The clock is at the left end (a right-to-left Windows taskbar).
    pub clock_left: bool,
}

impl Taskbar {
    /// The taskbar where it is when shown: at its edge of `monitor`, as
    /// thick as Windows reports it.
    pub fn shown_rect(&self, monitor: &Rect) -> Rect {
        let r = self.rect;
        match self.edge {
            Edge::Bottom => Rect::new(r.left, monitor.bottom - r.height(), r.right, monitor.bottom),
            Edge::Top => Rect::new(r.left, monitor.top, r.right, monitor.top + r.height()),
            Edge::Left => Rect::new(monitor.left, r.top, monitor.left + r.width(), r.bottom),
            Edge::Right => Rect::new(monitor.right - r.width(), r.top, monitor.right, r.bottom),
        }
    }

    /// Whether an auto-hide taskbar at `current` (its window rectangle now)
    /// is out, rather than tucked away as a thin line at the edge.
    pub fn is_out(&self, current: &Rect, monitor: &Rect, scale: f64) -> bool {
        let seen = current.intersect(monitor);
        let thick = if self.edge.horizontal() { seen.height() } else { seen.width() };
        thick > px(8, scale)
    }
}

/// Size of the floating box at `scale`.
pub fn box_size(scale: f64) -> (i32, i32) {
    (px(236, scale), px(36, scale))
}

/// How far the box keeps from the clock end of the taskbar, so Windows'
/// notifications (a column about 364 px wide above the clock) never cover it.
pub const CLOCK_CLEARANCE: i32 = 396;
/// Gap between the box and the taskbar.
pub const BOX_GAP: i32 = 8;

/// Where the floating box goes by default on `monitor` (the primary one):
/// just above the taskbar, beside the notification column; mirrored when
/// the clock is on the left; next to the tray for a taskbar on the side.
pub fn default_box_rect(monitor: &Rect, taskbar: Option<&Taskbar>, scale: f64) -> Rect {
    let (w, h) = box_size(scale);
    let gap = px(BOX_GAP, scale);
    let clear = px(CLOCK_CLEARANCE, scale);
    let r = match taskbar {
        Some(t) => {
            let tb = t.shown_rect(monitor);
            let x_near_clock = if t.clock_left { tb.left + clear } else { tb.right - clear - w };
            match t.edge {
                Edge::Bottom => Rect::at(x_near_clock, tb.top - gap - h, w, h),
                Edge::Top => Rect::at(x_near_clock, tb.bottom + gap, w, h),
                Edge::Left => Rect::at(tb.right + gap, tb.bottom - gap - h, w, h),
                Edge::Right => Rect::at(tb.left - gap - w, tb.bottom - gap - h, w, h),
            }
        }
        // No taskbar found: the bottom right corner (where it usually is).
        None => Rect::at(monitor.right - clear - w, monitor.bottom - px(48, scale) - gap - h, w, h),
    };
    r.clamp_into(monitor)
}

/// The box where the user left it, when the monitor it was on is still
/// there (same rectangle); kept on that monitor.
pub fn restore_box_rect(saved: &FloatPos, monitors: &[Rect], scale: f64) -> Option<Rect> {
    let [l, t, r, b] = saved.monitor;
    let mon = monitors.iter().find(|m| **m == Rect::new(l, t, r, b))?;
    let (w, h) = box_size(scale);
    Some(Rect::at(saved.x, saved.y, w, h).clamp_into(mon))
}

/// Where a popup opens from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Next to the floating box: above it (below it in the top half of the
    /// screen), lined up with its edge nearer the screen side.
    Box(Rect),
    /// Centred near the bottom of a monitor's work area (the hotkey).
    CenterBottom,
    /// Beside the clock (the tray icon).
    Tray { clock_left: bool },
}

/// The popup's rectangle: `w`×`h` placed at `anchor` inside `work` (the
/// work area of the monitor it opens on).
pub fn popup_rect(anchor: Anchor, work: &Rect, w: i32, h: i32, scale: f64) -> Rect {
    let gap = px(BOX_GAP, scale);
    let r = match anchor {
        Anchor::Box(b) => {
            let x = if b.center().0 >= work.center().0 { b.right - w } else { b.left };
            if b.center().1 < work.center().1 { Rect::at(x, b.bottom + gap, w, h) } else { Rect::at(x, b.top - gap - h, w, h) }
        }
        Anchor::CenterBottom => {
            let bottom = work.bottom - (work.height() / 6).min(px(160, scale));
            Rect::at(work.center().0 - w / 2, bottom - h, w, h)
        }
        Anchor::Tray { clock_left } => {
            let m = px(12, scale);
            let x = if clock_left { work.left + m } else { work.right - m - w };
            Rect::at(x, work.bottom - m - h, w, h)
        }
    };
    r.clamp_into(work)
}

/// The popup keeps its anchored edge when its height changes: the bottom
/// when it opened upwards (the usual case), else the top.
pub fn regrow(old: &Rect, new_h: i32, upward: bool, work: &Rect) -> Rect {
    let r = if upward { Rect::new(old.left, old.bottom - new_h, old.right, old.bottom) } else { Rect::at(old.left, old.top, old.width(), new_h) };
    r.clamp_into(work)
}

/// The x of a `w` wide item placed at `x` from the reading start of a row
/// `total` wide: unchanged left-to-right, mirrored right-to-left.
pub fn mirror_x(x: i32, w: i32, total: i32, rtl: bool) -> i32 {
    if rtl { total - x - w } else { x }
}

/// Whether `text` reads right to left by its first strong letter (Arabic,
/// Hebrew); `None` when it has none ("250*12", "؟").
pub fn first_strong_rtl(text: &str) -> Option<bool> {
    use unicode_bidi::{BidiClass, bidi_class};
    text.chars().find_map(|c| match bidi_class(c) {
        BidiClass::L => Some(false),
        BidiClass::R | BidiClass::AL => Some(true),
        _ => None,
    })
}

/// Keeps each quoted piece of a row title («…» or “…”, the typed query) in
/// its own reading order, so "250*12" or "5 kg to lb" quoted in an Arabic
/// sentence is not reordered by the sentence around it: the inside is put
/// in a left-to-right or right-to-left embedding, with its mark, by its
/// first strong letter; numbers and symbols alone in a left-to-right override.
pub fn embed_quoted(title: &str) -> String {
    let mut out = String::with_capacity(title.len() + 8);
    let mut rest = title;
    while let Some((i, open)) = rest.char_indices().find(|&(_, c)| c == '«' || c == '“') {
        let close = if open == '«' { '»' } else { '”' };
        let after = &rest[i + open.len_utf8()..];
        let Some(j) = after.find(close) else { break };
        let inner = &after[..j];
        out.push_str(&rest[..i + open.len_utf8()]);
        // Numbers and symbols alone are forced left to right: in an Arabic
        // sentence Windows takes their digits for Arabic ones, and even in
        // an embedding "250*12" came out as "12*250".
        out.push_str(match first_strong_rtl(inner) {
            Some(true) => "\u{202B}\u{200F}",
            Some(false) => "\u{202A}\u{200E}",
            None => "\u{202D}",
        });
        out.push_str(inner);
        out.push('\u{202C}');
        out.push(close);
        rest = &after[j + close.len_utf8()..];
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------ fading

/// How opaque the faded box is, as a share of its full look.
pub const IDLE_ALPHA: f32 = 0.18;
/// The cursor this close (logical px) wakes a fading box.
pub const WAKE_DISTANCE: i32 = 80;
/// A hidden box only wakes when the cursor is this close (its corner).
pub const HIDDEN_WAKE_DISTANCE: i32 = 24;
/// Time for a full fade in or out.
pub const FADE_MS: f32 = 150.0;

/// The opacity the box heads for: full when the cursor is near or `awake`
/// (the hotkey was used, the popup is open), else almost (fade) or fully
/// (hidden) transparent.
pub fn target_alpha(mode: FloatingMode, distance: i32, scale: f64, awake: bool) -> f32 {
    let near = match mode {
        FloatingMode::Fade => distance <= px(WAKE_DISTANCE, scale),
        FloatingMode::Hidden => distance <= px(HIDDEN_WAKE_DISTANCE, scale),
        FloatingMode::Off => return 0.0,
    };
    if near || awake {
        1.0
    } else if mode == FloatingMode::Fade {
        IDLE_ALPHA
    } else {
        0.0
    }
}

/// One animation step of `dt_ms` from `current` towards `target`.
pub fn step_alpha(current: f32, target: f32, dt_ms: f32) -> f32 {
    let step = (dt_ms.max(0.0) / FADE_MS).min(1.0);
    if current < target { (current + step).min(target) } else { (current - step).max(target) }
}

/// The 0–255 constant alpha for `alpha` (0.0–1.0).
pub fn alpha_byte(alpha: f32) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Whether a "do not disturb" state of Windows hides the box:
/// `SHQueryUserNotificationState` QUNS_BUSY (2, a full-screen app),
/// QUNS_RUNNING_D3D_FULL_SCREEN (3, a game or video) or
/// QUNS_PRESENTATION_MODE (4).
pub fn hides_for_notification_state(state: i32) -> bool {
    matches!(state, 2..=4)
}

// ------------------------------------------------------------------ results

/// The sections of the quick-search list, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Pinned,
    /// Recently opened files, shown while the box is empty.
    Recent,
    /// What to try when nothing was found ([`crate::suggest`]).
    Suggest,
    Best,
    Windows,
    Apps,
    Files,
    Settings,
    Answers,
    Commands,
    Web,
    /// Feedback rows and "open in InstaFind".
    More,
}

/// The kind of feedback a row opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackKind {
    Bug,
    Idea,
    /// The search found nothing (prefilled with the query).
    WrongResult,
}

/// What opening a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Launcher(Action),
    File { path: String, is_dir: bool },
    Feedback { kind: FeedbackKind, query: String },
    /// Continue the search in the full window.
    OpenInMain(String),
    /// Try a suggestion for a search that found nothing.
    Apply(crate::suggest::Suggestion),
}

impl Target {
    /// The file, folder or program behind the row, as pins store it.
    pub fn pin_target(&self) -> Option<&str> {
        match self {
            Target::File { path, .. } => Some(path),
            Target::Launcher(Action::Launch { target, .. }) => Some(target),
            _ => None,
        }
    }

    /// A file system path whose folder can be opened.
    pub fn path(&self) -> Option<&str> {
        match self {
            Target::File { path, .. } => Some(path),
            Target::Launcher(Action::Launch { target, .. }) if !target.to_ascii_lowercase().starts_with("shell:") => Some(target),
            _ => None,
        }
    }
}

/// The icon drawn at the start of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    /// The shell icon of this file, folder, exe or shortcut.
    Shell { path: String, is_dir: bool },
    /// A Segoe Fluent / MDL2 glyph.
    Glyph(char),
}

/// Glyphs (Segoe Fluent Icons / Segoe MDL2 Assets code points).
pub mod glyph {
    pub const SEARCH: char = '\u{E721}';
    pub const SETTINGS: char = '\u{E713}';
    pub const CALCULATOR: char = '\u{E8EF}';
    pub const POWER: char = '\u{E7E8}';
    pub const GLOBE: char = '\u{E774}';
    pub const WINDOW: char = '\u{E737}';
    pub const APPS: char = '\u{E71D}';
    pub const FEEDBACK: char = '\u{ED15}';
    pub const IDEA: char = '\u{EA80}';
    pub const OPEN_WINDOW: char = '\u{E8A7}';
}

/// One row of the quick-search list.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub group: Group,
    pub title: String,
    pub subtitle: String,
    pub target: Target,
    pub icon: Icon,
}

impl Row {
    /// A row for a launcher result.
    pub fn from_item(item: Item) -> Row {
        let group = match item.kind {
            Kind::Answer => Group::Answers,
            Kind::Window => Group::Windows,
            Kind::App => Group::Apps,
            Kind::Setting => Group::Settings,
            Kind::Command => Group::Commands,
            Kind::Web => Group::Web,
        };
        let icon = match &item.icon_path {
            Some(p) if !p.is_empty() => Icon::Shell { path: p.clone(), is_dir: false },
            _ => Icon::Glyph(match item.kind {
                Kind::Answer => glyph::CALCULATOR,
                Kind::Window => glyph::WINDOW,
                Kind::App => glyph::APPS,
                Kind::Setting => glyph::SETTINGS,
                Kind::Command => glyph::POWER,
                Kind::Web => glyph::GLOBE,
            }),
        };
        Row { group, title: item.title, subtitle: item.subtitle, target: Target::Launcher(item.action), icon }
    }

    /// A row for a file or folder found in the index.
    pub fn file(path: &str, is_dir: bool) -> Row {
        let t = path.trim_end_matches(['\\', '/']);
        let (folder, name) = match t.rfind(['\\', '/']) {
            Some(i) if i + 1 < t.len() => (&t[..i], &t[i + 1..]),
            _ => ("", path),
        };
        Row {
            group: Group::Files,
            title: name.to_string(),
            subtitle: folder.to_string(),
            target: Target::File { path: path.to_string(), is_dir },
            icon: Icon::Shell { path: path.to_string(), is_dir },
        }
    }
}

/// How many rows each section shows at most.
fn cap(group: Group) -> usize {
    match group {
        Group::Pinned => 3,
        Group::Recent => crate::suggest::RECENT_ROWS,
        Group::Suggest => 4,
        Group::Best => 1,
        Group::Windows => 3,
        Group::Apps => 4,
        Group::Files => 6,
        Group::Settings => 3,
        Group::Answers => 2,
        Group::Commands => 2,
        Group::Web => 2,
        Group::More => 3,
    }
}

/// Most result rows shown at once (the "more" rows come on top).
pub const MAX_RESULTS: usize = 12;

/// The words that offer the feedback rows, in both languages (compared
/// after [`crate::normalize::normalize`]).
const BUG_WORDS: &[&str] = &["مشكلة", "مشكل", "خلل", "problem", "bug", "issue"];
const IDEA_WORDS: &[&str] = &["اقتراح", "اقترح", "فكرة", "suggest", "suggestion", "idea", "feature"];

/// Which feedback rows a query asks for: a word of it starts one of the
/// trigger words (three letters at least) or contains one.
pub fn feedback_kinds(query: &str) -> Vec<FeedbackKind> {
    let q = crate::normalize::normalize(query);
    let words: Vec<&str> = q.split_whitespace().collect();
    let hit = |list: &[&str]| {
        list.iter().any(|t| {
            let t = crate::normalize::normalize(t);
            words.iter().any(|w| w.contains(t.as_str()) || (w.chars().count() >= 3 && t.starts_with(w)))
        })
    };
    let mut v = Vec::new();
    if hit(BUG_WORDS) {
        v.push(FeedbackKind::Bug);
    }
    if hit(IDEA_WORDS) {
        v.push(FeedbackKind::Idea);
    }
    v
}

/// The rows of one query, in display order.
pub struct Parts {
    pub query: String,
    /// Pinned favourites that match, as rows (any group; they are moved to
    /// [`Group::Pinned`]).
    pub pins: Vec<Row>,
    /// Launcher results, best first (`Catalog::query`).
    pub items: Vec<Item>,
    /// Files and folders, best first.
    pub files: Vec<Row>,
    /// The file search for this query has finished (so "nothing found" is
    /// final, not just not yet known).
    pub files_done: bool,
}

/// Puts together the list: pinned matches, the best result, then windows,
/// apps, files, settings, answers, commands and web, each section capped,
/// at most [`MAX_RESULTS`] in all, then the feedback rows and "open in
/// InstaFind".
pub fn arrange(parts: Parts) -> Vec<Row> {
    let query = parts.query.trim().to_string();
    let mut pins: Vec<Row> = parts.pins.into_iter().map(|r| Row { group: Group::Pinned, ..r }).collect();
    pins.truncate(cap(Group::Pinned));
    let pinned = |t: &Target| pins.iter().any(|p| p.target.pin_target().is_some_and(|a| t.pin_target().is_some_and(|b| a.eq_ignore_ascii_case(b))));
    // Apps matched strongly (a name or its start, Arabic ones too, or a habit) lead over files.
    let strong: Vec<Target> = parts
        .items
        .iter()
        .filter(|i| i.kind == Kind::App && i.score >= crate::launcher::STRONG_SCORE)
        .map(|i| Target::Launcher(i.action.clone()))
        .collect();
    let answered = parts.items.iter().any(|i| i.kind == Kind::Answer);
    let mut items: Vec<Row> = parts.items.into_iter().map(Row::from_item).filter(|r| !pinned(&r.target)).collect();
    let mut files: Vec<Row> = parts.files.into_iter().filter(|r| !pinned(&r.target)).collect();
    let nq = crate::normalize::normalize(&query);
    if answered {
        // "2+2" is a sum: only files named with the whole of it, not every name with a 2.
        files.retain(|r| crate::normalize::normalize(&r.title).contains(nq.as_str()));
    }
    let found = !pins.is_empty() || !files.is_empty() || items.iter().any(|r| r.group != Group::Web);

    // The best result, unless pinned favourites already lead.
    let mut best: Option<Row> = None;
    if pins.is_empty() && !query.is_empty() {
        let starts = |r: &Row| crate::normalize::normalize(&r.title).starts_with(&nq) || strong.contains(&r.target);
        let has_app = items.iter().any(|r| r.group == Group::Apps);
        if let Some(i) = items.iter().position(|r| r.group == Group::Answers) {
            best = Some(items.remove(i));
        } else if let Some(i) = items.iter().position(|r| r.group != Group::Web && starts(r)) {
            best = Some(items.remove(i));
        } else if !files.is_empty() && (!has_app || names_query(&files[0].title, &nq)) {
            best = Some(files.remove(0));
        } else if let Some(i) = items.iter().position(|r| r.group == Group::Apps) {
            // The files only resemble the query ("notepda"): an app found
            // despite the typo comes first.
            best = Some(items.remove(i));
        } else if let Some(i) = items.iter().position(|r| r.group != Group::Web) {
            best = Some(items.remove(i));
        }
    }

    let mut sections: Vec<Vec<Row>> = Vec::new();
    sections.push(pins);
    sections.push(best.into_iter().map(|r| Row { group: Group::Best, ..r }).collect());
    for g in [Group::Windows, Group::Apps, Group::Files, Group::Settings, Group::Answers, Group::Commands, Group::Web] {
        let mut v: Vec<Row> = if g == Group::Files {
            std::mem::take(&mut files)
        } else {
            let (mine, rest): (Vec<Row>, Vec<Row>) = std::mem::take(&mut items).into_iter().partition(|r| r.group == g);
            items = rest;
            mine
        };
        v.truncate(cap(g));
        sections.push(v);
    }
    // Over the limit: the biggest section gives up its last row, until it fits.
    while sections.iter().map(Vec::len).sum::<usize>() > MAX_RESULTS {
        let Some(big) = sections.iter_mut().filter(|s| s.len() > 1).max_by_key(|s| s.len()) else { break };
        big.pop();
    }
    let mut rows: Vec<Row> = sections.into_iter().flatten().collect();

    if !query.is_empty() {
        for kind in feedback_kinds(&query) {
            rows.push(feedback_row(kind, &query));
        }
        if !found && parts.files_done {
            rows.push(feedback_row(FeedbackKind::WrongResult, &query));
        }
        rows.push(Row {
            group: Group::More,
            title: String::new(),
            subtitle: String::new(),
            target: Target::OpenInMain(query.clone()),
            icon: Icon::Glyph(glyph::OPEN_WINDOW),
        });
    }
    rows
}

/// Whether a file name holds every word of the normalised query as typed,
/// rather than only resembling it (a typo match). Words with search syntax
/// ("ext:pdf", "!old", "a|b", wildcards) count as held.
fn names_query(title: &str, nq: &str) -> bool {
    let name = crate::normalize::normalize(title);
    nq.split_whitespace().all(|w| w.starts_with(['!', '-']) || w.contains([':', '|', '*', '?', '"']) || name.contains(w))
}

/// A feedback row; its texts are filled by the UI in the interface language.
fn feedback_row(kind: FeedbackKind, query: &str) -> Row {
    Row {
        group: Group::More,
        title: String::new(),
        subtitle: String::new(),
        target: Target::Feedback { kind, query: query.to_string() },
        icon: Icon::Glyph(if kind == FeedbackKind::Idea { glyph::IDEA } else { glyph::FEEDBACK }),
    }
}

/// The row to select after the list changed: the same target if it is still
/// there, else the first row.
pub fn reselect(rows: &[Row], previous: Option<&Target>) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    Some(previous.and_then(|t| rows.iter().position(|r| &r.target == t)).unwrap_or(0))
}

/// Moves the selection by `delta` rows, wrapping around.
pub fn move_selection(current: Option<usize>, delta: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let n = len as i64;
    let cur = current.map_or(if delta > 0 { -1 } else { 0 }, |c| c as i64);
    Some((cur + delta as i64).rem_euclid(n) as usize)
}

// ------------------------------------------------------------------ list layout

/// What occupies a stretch of the popup's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    /// The name of a section.
    Header(Group),
    /// Result row `i`.
    Row(usize),
    /// A thin line before the "more" rows.
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub kind: SlotKind,
    pub y: i32,
    pub h: i32,
}

/// Heights (pixels) of the parts of the list.
#[derive(Clone, Copy, Debug)]
pub struct ListMetrics {
    pub row: i32,
    /// "More" rows (feedback, open in InstaFind) are lower.
    pub more: i32,
    pub header: i32,
    pub separator: i32,
    /// Space above the first and below the last slot.
    pub pad: i32,
}

/// Lays out rows of `groups` from the top: a header where a section starts,
/// a separator before the "more" rows. Returns the slots and the height.
pub fn layout_list(groups: &[Group], m: &ListMetrics) -> (Vec<Slot>, i32) {
    let mut slots = Vec::new();
    let mut y = m.pad;
    let mut last: Option<Group> = None;
    for (i, &g) in groups.iter().enumerate() {
        if last != Some(g) {
            if g == Group::More {
                if last.is_some() {
                    slots.push(Slot { kind: SlotKind::Separator, y, h: m.separator });
                    y += m.separator;
                }
            } else {
                slots.push(Slot { kind: SlotKind::Header(g), y, h: m.header });
                y += m.header;
            }
            last = Some(g);
        }
        let h = if g == Group::More { m.more } else { m.row };
        slots.push(Slot { kind: SlotKind::Row(i), y, h });
        y += h;
    }
    (slots, if groups.is_empty() { 0 } else { y + m.pad })
}

/// Drops result rows (the last ones before the "more" rows) until the list
/// is at most `max_h` tall; the "more" rows always stay.
pub fn fit_rows(rows: &mut Vec<Row>, m: &ListMetrics, max_h: i32) {
    loop {
        let groups: Vec<Group> = rows.iter().map(|r| r.group).collect();
        if layout_list(&groups, m).1 <= max_h {
            return;
        }
        match rows.iter().rposition(|r| r.group != Group::More) {
            Some(i) => {
                rows.remove(i);
            }
            None => return,
        }
    }
}

/// The result row at height `y` of the list, if any.
pub fn row_at(slots: &[Slot], y: i32) -> Option<usize> {
    slots.iter().find_map(|s| match s.kind {
        SlotKind::Row(i) if y >= s.y && y < s.y + s.h => Some(i),
        _ => None,
    })
}

// ------------------------------------------------------------------ pixels

/// Premultiplied BGRA pixels, as Windows' 32-bit DIBs and
/// `UpdateLayeredWindow` want them: `0xAARRGGBB`.
pub type Pixel = u32;

/// `0x00RRGGBB` colour with opacity `alpha` (0–255), premultiplied.
pub fn premul(rgb: u32, alpha: u8) -> Pixel {
    let a = alpha as u32;
    let ch = |shift: u32| (((rgb >> shift) & 0xFF) * a + 127) / 255;
    (a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// `top` over `bottom` (both premultiplied).
pub fn over(top: Pixel, bottom: Pixel) -> Pixel {
    let ta = top >> 24;
    if ta == 255 {
        return top;
    }
    if ta == 0 && top == 0 {
        return bottom;
    }
    let inv = 255 - ta;
    let ch = |shift: u32| {
        let t = (top >> shift) & 0xFF;
        let b = (bottom >> shift) & 0xFF;
        (t + (b * inv + 127) / 255).min(255)
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Scales a premultiplied pixel's opacity by `coverage` (0–255).
pub fn fade(p: Pixel, coverage: u8) -> Pixel {
    let c = coverage as u32;
    let ch = |shift: u32| ((((p >> shift) & 0xFF) * c + 127) / 255) << shift;
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// Recovers colour and opacity from the same drawing made twice by GDI
/// (which knows no alpha): once on black and once on white. Where the two
/// differ by `d`, the drawing let `d/255` of the background through.
pub fn matte(on_black: Pixel, on_white: Pixel) -> Pixel {
    let ch = |p: Pixel, shift: u32| (p >> shift) & 0xFF;
    let mut alpha = 0;
    for shift in [0, 8, 16] {
        let diff = ch(on_white, shift).saturating_sub(ch(on_black, shift));
        alpha = alpha.max(255 - diff);
    }
    if alpha == 0 {
        return 0;
    }
    let c = |shift: u32| ch(on_black, shift).min(alpha);
    (alpha << 24) | (c(16) << 16) | (c(8) << 8) | c(0)
}

/// How much of pixel (`x`, `y`) a rounded rectangle `w`×`h` with corner
/// radius `r` covers (0–255), antialiased over about one pixel.
pub fn rounded_coverage(x: i32, y: i32, w: i32, h: i32, r: f32) -> u8 {
    let (px_, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (w, h) = (w as f32, h as f32);
    let r = r.clamp(0.0, w.min(h) / 2.0);
    // Signed distance to the rounded rectangle (negative inside).
    let qx = (px_ - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (py - h / 2.0).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let d = outside + qx.max(qy).min(0.0) - r;
    ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A monochrome magnifier, the tray icon: `size`×`size` premultiplied
/// pixels of `rgb`, top row first. Drawn by distance, so every size is crisp.
pub fn magnifier_icon(size: i32, rgb: u32) -> Vec<Pixel> {
    let s = size.max(1) as f32;
    let (cx, cy) = (s * 0.42, s * 0.42);
    let radius = s * 0.27;
    let stroke = (s * 0.11).max(1.3);
    // Handle: from the ring towards the bottom-right corner.
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let (hx0, hy0) = (cx + (radius + stroke * 0.3) * k, cy + (radius + stroke * 0.3) * k);
    let (hx1, hy1) = (s * 0.86, s * 0.86);
    let handle_w = stroke * 1.25;
    let mut out = Vec::with_capacity((size.max(1) * size.max(1)) as usize);
    for y in 0..size.max(1) {
        for x in 0..size.max(1) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let ring = ((fx - cx).hypot(fy - cy) - radius).abs() - stroke / 2.0;
            // Distance to the handle segment.
            let (vx, vy) = (hx1 - hx0, hy1 - hy0);
            let t = (((fx - hx0) * vx + (fy - hy0) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
            let seg = (fx - (hx0 + t * vx)).hypot(fy - (hy0 + t * vy)) - handle_w / 2.0;
            let d = ring.min(seg);
            let cov = ((0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8;
            out.push(premul(rgb, cov));
        }
    }
    out
}

// ------------------------------------------------------------------ colours

/// The colours of the popup and the box for a light or dark theme
/// (`0x00RRGGBB`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Solid background (Windows 10, or no transparency effects).
    pub background: u32,
    pub text: u32,
    pub muted: u32,
    /// The selected row, drawn over the background with `select_alpha`.
    pub select: u32,
    pub select_alpha: u8,
    /// Thin separators and the box's outline.
    pub line: u32,
    /// The floating box's tint and its opacity at full look.
    pub box_tint: u32,
    pub box_alpha: u8,
    pub accent: u32,
}

impl Palette {
    pub fn new(dark: bool) -> Palette {
        if dark {
            Palette {
                background: 0x20_20_20,
                text: 0xF2_F2_F2,
                muted: 0xA8_A8_A8,
                select: 0xFF_FF_FF,
                select_alpha: 26,
                line: 0x3C_3C_3C,
                box_tint: 0x2B_2B_2B,
                box_alpha: 200,
                accent: 0x60_CD_FF,
            }
        } else {
            Palette {
                background: 0xF9_F9_F9,
                text: 0x1A_1A_1A,
                muted: 0x5F_5F_5F,
                select: 0x00_00_00,
                select_alpha: 15,
                line: 0xE0_E0_E0,
                box_tint: 0xF7_F7_F7,
                box_alpha: 205,
                accent: 0x00_5F_B8,
            }
        }
    }
}

/// Windows 11 22H2 (build 22621) has the transient (acrylic) backdrop for
/// any window; earlier builds get a solid background.
pub fn has_backdrop(build: u32) -> bool {
    build >= 22621
}

/// Windows 11 (build 22000) rounds window corners on request.
pub fn has_round_corners(build: u32) -> bool {
    build >= 22000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_query_keeps_its_reading_order() {
        assert_eq!(first_strong_rtl("250*12"), None);
        assert_eq!(first_strong_rtl("5 kg to lb"), Some(false));
        assert_eq!(first_strong_rtl("12 تقرير"), Some(true));
        assert_eq!(embed_quoted("افتح «250*12» في نافذة البحث الكاملة"), "افتح «\u{202D}250*12\u{202C}» في نافذة البحث الكاملة");
        assert_eq!(embed_quoted("ابحث في يوتيوب عن «قرآن»"), "ابحث في يوتيوب عن «\u{202B}\u{200F}قرآن\u{202C}»");
        assert_eq!(embed_quoted("Open “5 kg” and “x”"), "Open “\u{202A}\u{200E}5 kg\u{202C}” and “\u{202A}\u{200E}x\u{202C}”");
        // Nothing quoted, or a quote left open: unchanged.
        for t in ["Wi-Fi", "«open", "a » b «", "", "«»"] {
            let e = embed_quoted(t);
            assert_eq!(e.replace(['\u{202A}', '\u{200E}', '\u{202C}', '\u{202D}'], ""), t);
        }
    }
    use crate::launcher::SystemCommand;

    const MON: Rect = Rect::new(0, 0, 1920, 1080);

    fn bottom_bar(clock_left: bool) -> Taskbar {
        Taskbar { rect: Rect::new(0, 1032, 1920, 1080), edge: Edge::Bottom, clock_left }
    }

    #[test]
    fn rect_basics() {
        let r = Rect::at(10, 20, 100, 50);
        assert_eq!((r.width(), r.height(), r.center()), (100, 50, (60, 45)));
        assert_eq!(r.distance(50, 40), 0);
        assert_eq!(r.distance(0, 40), 10);
        assert_eq!(r.distance(112, 73), 5); // 3 right, 4 below
        assert_eq!(Rect::at(-50, -50, 100, 100).clamp_into(&MON), Rect::at(0, 0, 100, 100));
        assert_eq!(Rect::at(1900, 1000, 100, 100).clamp_into(&MON), Rect::at(1820, 980, 100, 100));
        // Bigger than the screen: cut to it.
        assert_eq!(Rect::at(5, 5, 5000, 50).clamp_into(&MON), Rect::at(0, 5, 1920, 50));
        assert!(Rect::at(0, 0, 10, 10).intersect(&Rect::at(20, 20, 5, 5)).is_empty());
        assert_eq!(Rect::at(0, 0, 10, 10).intersect(&Rect::at(5, 5, 10, 10)), Rect::at(5, 5, 5, 5));
        assert_eq!(px(236, 1.5), 354);
        assert_eq!(mirror_x(10, 20, 100, false), 10);
        assert_eq!(mirror_x(10, 20, 100, true), 70);
    }

    #[test]
    fn box_sits_beside_the_notifications() {
        // 100 %: 236×36, 8 px above the taskbar, 396 px from the clock end.
        let r = default_box_rect(&MON, Some(&bottom_bar(false)), 1.0);
        assert_eq!(r, Rect::at(1920 - 396 - 236, 1032 - 8 - 36, 236, 36));
        // Arabic Windows: the clock is on the left, so is the box.
        let r = default_box_rect(&MON, Some(&bottom_bar(true)), 1.0);
        assert_eq!(r, Rect::at(396, 1032 - 8 - 36, 236, 36));
        // 150 %: everything scales.
        let tb = Taskbar { rect: Rect::new(0, 1008, 1920, 1080), ..bottom_bar(false) };
        let r = default_box_rect(&MON, Some(&tb), 1.5);
        assert_eq!(r, Rect::at(1920 - 594 - 354, 1008 - 12 - 54, 354, 54));
    }

    #[test]
    fn taskbar_edges() {
        assert_eq!([0, 1, 2, 3, 9].map(Edge::from_abe), [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom, Edge::Bottom]);
        assert!(Edge::Bottom.horizontal() && !Edge::Left.horizontal());
        // Every glyph is a private-use code point of the icon fonts, and they differ.
        let all = [glyph::SEARCH, glyph::SETTINGS, glyph::CALCULATOR, glyph::POWER, glyph::GLOBE, glyph::WINDOW, glyph::APPS, glyph::FEEDBACK, glyph::IDEA, glyph::OPEN_WINDOW];
        assert!(all.iter().all(|g| ('\u{E000}'..='\u{F8FF}').contains(g)));
        assert!(all.iter().enumerate().all(|(i, g)| !all[i + 1..].contains(g)));
    }

    #[test]
    fn box_follows_the_taskbar_edge() {
        let top = Taskbar { rect: Rect::new(0, 0, 1920, 40), edge: Edge::Top, clock_left: false };
        assert_eq!(default_box_rect(&MON, Some(&top), 1.0), Rect::at(1288, 48, 236, 36));
        let left = Taskbar { rect: Rect::new(0, 0, 62, 1080), edge: Edge::Left, clock_left: false };
        assert_eq!(default_box_rect(&MON, Some(&left), 1.0), Rect::at(70, 1036, 236, 36));
        let right = Taskbar { rect: Rect::new(1858, 0, 1920, 1080), edge: Edge::Right, clock_left: false };
        assert_eq!(default_box_rect(&MON, Some(&right), 1.0), Rect::at(1858 - 8 - 236, 1036, 236, 36));
        // A hidden auto-hide taskbar (only 2 px on screen) counts where it shows.
        let hidden = Taskbar { rect: Rect::new(0, 1078, 1920, 1126), ..bottom_bar(false) };
        assert_eq!(hidden.shown_rect(&MON), Rect::new(0, 1032, 1920, 1080));
        assert_eq!(default_box_rect(&MON, Some(&hidden), 1.0), default_box_rect(&MON, Some(&bottom_bar(false)), 1.0));
        assert!(!hidden.is_out(&Rect::new(0, 1078, 1920, 1126), &MON, 1.0));
        assert!(hidden.is_out(&Rect::new(0, 1032, 1920, 1080), &MON, 1.0));
        // A second monitor on the left, at negative coordinates.
        let mon2 = Rect::new(-1280, 0, 0, 1024);
        let tb2 = Taskbar { rect: Rect::new(-1280, 984, 0, 1024), edge: Edge::Bottom, clock_left: false };
        let r = default_box_rect(&mon2, Some(&tb2), 1.0);
        assert_eq!(r, Rect::at(-396 - 236, 984 - 44, 236, 36));
        // A tiny screen: kept on it.
        let small = Rect::new(0, 0, 500, 300);
        let tb3 = Taskbar { rect: Rect::new(0, 260, 500, 300), edge: Edge::Bottom, clock_left: false };
        let r = default_box_rect(&small, Some(&tb3), 1.0);
        assert!(r.left >= 0 && r.right <= 500 && r.bottom <= 300);
        assert!(default_box_rect(&MON, None, 1.0).bottom <= 1080);
    }

    #[test]
    fn saved_position_needs_its_monitor() {
        let saved = FloatPos { monitor: [0, 0, 1920, 1080], x: 100, y: 200 };
        assert_eq!(restore_box_rect(&saved, &[MON], 1.0), Some(Rect::at(100, 200, 236, 36)));
        assert_eq!(restore_box_rect(&saved, &[Rect::new(0, 0, 2560, 1440)], 1.0), None);
        // Dragged partly off: brought back on.
        let off = FloatPos { x: 1900, y: 1070, ..saved };
        assert_eq!(restore_box_rect(&off, &[MON], 1.0), Some(Rect::at(1920 - 236, 1080 - 36, 236, 36)));
    }

    #[test]
    fn popup_opens_next_to_its_anchor() {
        let work = Rect::new(0, 0, 1920, 1032);
        let b = default_box_rect(&MON, Some(&bottom_bar(false)), 1.0);
        let p = popup_rect(Anchor::Box(b), &work, 460, 300, 1.0);
        // Right above the box, sharing its right edge (the clock side).
        assert_eq!((p.right, p.bottom, p.width(), p.height()), (b.right, b.top - 8, 460, 300));
        // Clock on the left: shares the box's left edge.
        let b = default_box_rect(&MON, Some(&bottom_bar(true)), 1.0);
        let p = popup_rect(Anchor::Box(b), &work, 460, 300, 1.0);
        assert_eq!((p.left, p.bottom), (b.left, b.top - 8));
        // A box near the top opens the popup below it.
        let b = Rect::at(1500, 48, 236, 36);
        assert_eq!(popup_rect(Anchor::Box(b), &work, 460, 300, 1.0).top, 92);
        // The hotkey: centred, near the bottom.
        let p = popup_rect(Anchor::CenterBottom, &work, 460, 300, 1.0);
        assert_eq!((p.left, p.bottom), (730, 1032 - 160));
        let p = popup_rect(Anchor::Tray { clock_left: false }, &work, 460, 300, 1.0);
        assert_eq!((p.right, p.bottom), (1920 - 12, 1032 - 12));
        let p = popup_rect(Anchor::Tray { clock_left: true }, &work, 460, 300, 1.0);
        assert_eq!(p.left, 12);
        // Taller than the screen: kept inside.
        let p = popup_rect(Anchor::CenterBottom, &work, 460, 5000, 1.0);
        assert_eq!((p.top, p.bottom), (0, 1032));
        // Growing keeps the bottom edge.
        let old = Rect::new(100, 500, 560, 800);
        assert_eq!(regrow(&old, 400, true, &work), Rect::new(100, 400, 560, 800));
        assert_eq!(regrow(&old, 400, false, &work), Rect::new(100, 500, 560, 900));
    }

    #[test]
    fn fading() {
        use FloatingMode::*;
        assert_eq!(target_alpha(Fade, 500, 1.0, false), IDLE_ALPHA);
        assert_eq!(target_alpha(Fade, 80, 1.0, false), 1.0);
        assert_eq!(target_alpha(Fade, 100, 1.5, false), 1.0); // 80 px at 150 % is 120
        assert_eq!(target_alpha(Fade, 500, 1.0, true), 1.0);
        assert_eq!(target_alpha(Hidden, 60, 1.0, false), 0.0);
        assert_eq!(target_alpha(Hidden, 0, 1.0, false), 1.0);
        assert_eq!(target_alpha(Off, 0, 1.0, true), 0.0);
        // 150 ms for a full fade; never overshoots.
        assert!((step_alpha(0.0, 1.0, 75.0) - 0.5).abs() < 1e-6);
        assert_eq!(step_alpha(0.9, 1.0, 75.0), 1.0);
        assert_eq!(step_alpha(1.0, IDLE_ALPHA, 1000.0), IDLE_ALPHA);
        assert_eq!(step_alpha(0.5, 0.5, 16.0), 0.5);
        assert_eq!(step_alpha(0.5, 0.0, -5.0), 0.5);
        assert_eq!((alpha_byte(1.0), alpha_byte(0.0), alpha_byte(2.0), alpha_byte(IDLE_ALPHA)), (255, 0, 255, 46));
        assert!(hides_for_notification_state(2) && hides_for_notification_state(3) && hides_for_notification_state(4));
        assert!(!hides_for_notification_state(5) && !hides_for_notification_state(1) && !hides_for_notification_state(7));
    }

    fn item(kind: Kind, title: &str, action: Action) -> Item {
        Item { kind, title: title.into(), subtitle: String::new(), action, score: 0, icon_path: None }
    }

    fn launch(t: &str) -> Action {
        Action::Launch { target: t.into(), args: String::new() }
    }

    fn groups(rows: &[Row]) -> Vec<Group> {
        rows.iter().map(|r| r.group).collect()
    }

    #[test]
    fn arrangement_and_order() {
        let items = vec![
            item(Kind::Web, "Search the web for word", Action::OpenUri("https://x".into())),
            item(Kind::Setting, "Display", Action::OpenUri("ms-settings:display".into())),
            item(Kind::App, "Word", launch(r"C:\Word.lnk")),
            item(Kind::Window, "Report - Word", Action::FocusWindow(5)),
            item(Kind::Command, "Lock", Action::System(SystemCommand::Lock)),
        ];
        let files = vec![Row::file(r"D:\docs\word list.txt", false), Row::file(r"D:\docs\words", true)];
        let rows = arrange(Parts { query: "word".into(), pins: vec![], items, files, files_done: true });
        // "Word" starts with the query: it is the best result.
        assert_eq!(rows[0].title, "Word");
        assert_eq!(
            groups(&rows),
            vec![Group::Best, Group::Windows, Group::Files, Group::Files, Group::Settings, Group::Commands, Group::Web, Group::More]
        );
        assert_eq!(rows.last().unwrap().target, Target::OpenInMain("word".into()));
        // A file row: name and folder.
        assert_eq!((rows[2].title.as_str(), rows[2].subtitle.as_str()), ("word list.txt", r"D:\docs"));
        assert_eq!(rows[3].title, "words");
        assert_eq!(rows[3].icon, Icon::Shell { path: r"D:\docs\words".into(), is_dir: true });
    }

    #[test]
    fn best_result_rules() {
        let calc = item(Kind::Answer, "= 4", Action::CopyText("4".into()));
        let app = item(Kind::App, "Calculator", launch("shell:AppsFolder\\calc"));
        let rows = arrange(Parts { query: "2+2".into(), pins: vec![], items: vec![app.clone(), calc], files: vec![], files_done: false });
        assert_eq!(rows[0].group, Group::Best);
        assert_eq!(rows[0].target, Target::Launcher(Action::CopyText("4".into())));
        // No title starts with the query: the first file wins over apps.
        let files = vec![Row::file(r"C:\a\calc notes.txt", false)];
        let rows = arrange(Parts { query: "notes".into(), pins: vec![], items: vec![app.clone()], files, files_done: true });
        assert_eq!((rows[0].group, rows[0].title.as_str()), (Group::Best, "calc notes.txt"));
        assert_eq!(rows[1].group, Group::Apps);
        // Arabic: letter forms do not matter for "starts with".
        let ar = item(Kind::Setting, "إعدادات العرض", Action::OpenUri("ms-settings:display".into()));
        let rows = arrange(Parts { query: "اعدادات".into(), pins: vec![], items: vec![app, ar], files: vec![], files_done: true });
        assert_eq!(rows[0].title, "إعدادات العرض");
        // Only a web shortcut: never "best".
        let web = item(Kind::Web, "YouTube", Action::OpenUri("https://youtube.com".into()));
        let rows = arrange(Parts { query: "you".into(), pins: vec![], items: vec![web], files: vec![], files_done: true });
        assert_eq!(rows[0].group, Group::Web);
    }

    #[test]
    fn pins_lead_and_are_not_repeated() {
        let pin = Row { group: Group::Apps, ..Row::from_item(item(Kind::App, "Word", launch(r"C:\Word.lnk"))) };
        let items = vec![item(Kind::App, "Word", launch(r"c:\word.lnk")), item(Kind::App, "WordPad", launch(r"C:\WordPad.lnk"))];
        let files = vec![Row::file(r"D:\x\word.txt", false)];
        let rows = arrange(Parts { query: "word".into(), pins: vec![pin], items, files, files_done: true });
        assert_eq!(groups(&rows), vec![Group::Pinned, Group::Apps, Group::Files, Group::More]);
        assert_eq!(rows[1].title, "WordPad");
        // A pinned file is not listed again among the files.
        let pin = Row::file(r"D:\x\word.txt", false);
        let rows = arrange(Parts { query: "word".into(), pins: vec![pin], items: vec![], files: vec![Row::file(r"d:\X\WORD.txt", false)], files_done: true });
        assert_eq!(groups(&rows), vec![Group::Pinned, Group::More]);
        // Empty query: only the pins, no "open in InstaFind".
        let pin = Row::file(r"D:\x\word.txt", false);
        let rows = arrange(Parts { query: " ".into(), pins: vec![pin], items: vec![], files: vec![], files_done: true });
        assert_eq!(groups(&rows), vec![Group::Pinned]);
        assert!(arrange(Parts { query: String::new(), pins: vec![], items: vec![], files: vec![], files_done: true }).is_empty());
    }

    #[test]
    fn caps_and_limit() {
        let items: Vec<Item> = (0..10).map(|i| item(Kind::App, &format!("zapp{i}"), launch(&format!("a{i}")))).chain((0..10).map(|i| item(Kind::Setting, &format!("zset{i}"), Action::OpenUri(format!("s{i}"))))).collect();
        let files: Vec<Row> = (0..20).map(|i| Row::file(&format!(r"C:\f{i}.txt"), false)).collect();
        let rows = arrange(Parts { query: "q".into(), pins: vec![], items, files, files_done: true });
        let n = |g| rows.iter().filter(|r| r.group == g).count();
        assert_eq!(rows.iter().filter(|r| r.group != Group::More).count(), MAX_RESULTS);
        assert!(n(Group::Apps) <= 4 && n(Group::Files) <= 6 && n(Group::Settings) <= 3);
        assert_eq!(n(Group::Best), 1);
        // Sections stay in order.
        let g = groups(&rows);
        assert!(g.windows(2).all(|w| w[0] <= w[1]), "{g:?}");
    }

    #[test]
    fn feedback_rows() {
        assert_eq!(feedback_kinds("مشكلة"), vec![FeedbackKind::Bug]);
        assert_eq!(feedback_kinds("مشكله في البحث"), vec![FeedbackKind::Bug]);
        assert_eq!(feedback_kinds("اقتراح"), vec![FeedbackKind::Idea]);
        assert_eq!(feedback_kinds("Problem"), vec![FeedbackKind::Bug]);
        assert_eq!(feedback_kinds("sugg"), vec![FeedbackKind::Idea]);
        assert_eq!(feedback_kinds("suggestions"), vec![FeedbackKind::Idea]);
        assert!(feedback_kinds("su").is_empty());
        assert!(feedback_kinds("report.docx").is_empty());
        assert!(feedback_kinds("").is_empty());
        let rows = arrange(Parts { query: "problem".into(), pins: vec![], items: vec![], files: vec![], files_done: true });
        let t: Vec<&Target> = rows.iter().map(|r| &r.target).collect();
        assert_eq!(
            t,
            vec![
                &Target::Feedback { kind: FeedbackKind::Bug, query: "problem".into() },
                &Target::Feedback { kind: FeedbackKind::WrongResult, query: "problem".into() },
                &Target::OpenInMain("problem".into()),
            ]
        );
        // Nothing found, but the file search is still running: no "wrong result" yet.
        let rows = arrange(Parts { query: "zzqx".into(), pins: vec![], items: vec![], files: vec![], files_done: false });
        assert_eq!(rows.len(), 1);
        // Only the web fallback found: still "nothing found".
        let web = item(Kind::Web, "Search the web", Action::OpenUri("https://x".into()));
        let rows = arrange(Parts { query: "zzqx".into(), pins: vec![], items: vec![web], files: vec![], files_done: true });
        assert!(rows.iter().any(|r| matches!(r.target, Target::Feedback { kind: FeedbackKind::WrongResult, .. })));
    }

    #[test]
    fn launcher_results_feed_the_list() {
        let catalog = crate::launcher::Catalog::default();
        let launches = |_: &str| 0;
        let ctx = crate::launcher::Context { english: false, windows_build: 22631, windows: &[], launches: &launches, picked: &launches, today: (2026, 11, 1) };
        let items = catalog.query("word", &ctx, 20);
        let rows = arrange(Parts { query: "word".into(), pins: vec![], items, files: vec![Row::file("C:\\word.txt", false)], files_done: true });
        assert_eq!(rows[0].group, Group::Best);
        assert_eq!(rows.last().map(|r| &r.target), Some(&Target::OpenInMain("word".into())));
    }

    fn catalog_items(names: &[&str], q: &str) -> Vec<Item> {
        let apps = names.iter().map(|n| crate::launcher::apps::App::new(n.to_string(), format!("shell:AppsFolder\\{n}"), None)).collect();
        let catalog = crate::launcher::Catalog::new(apps, Vec::new());
        let none = |_: &str| 0;
        let ctx = crate::launcher::Context { english: true, windows_build: 22631, windows: &[], launches: &none, picked: &none, today: (2026, 11, 1) };
        catalog.query(q, &ctx, 40)
    }

    #[test]
    fn typo_app_before_files_that_only_resemble_the_query() {
        let items = catalog_items(&["Notepad", "Notepad++", "Paint"], "notepda");
        assert_eq!(items.first().map(|i| i.title.as_str()), Some("Notepad"));
        // The file search found these by its typo fallback.
        let files = vec![Row::file(r"C:\notes\notepad tips.txt", false), Row::file(r"C:\notepad.ini", false)];
        let rows = arrange(Parts { query: "notepda".into(), pins: vec![], items, files, files_done: true });
        assert_eq!((rows[0].group, rows[0].title.as_str()), (Group::Best, "Notepad"));
        let first_file = rows.iter().position(|r| r.group == Group::Files).unwrap();
        let last_app = rows.iter().rposition(|r| r.group == Group::Apps).unwrap();
        assert!(last_app < first_file);
        // A file really named with the query still leads.
        let items = catalog_items(&["Notepad"], "notepda");
        let files = vec![Row::file(r"C:\notepda.txt", false)];
        let rows = arrange(Parts { query: "notepda".into(), pins: vec![], items, files, files_done: true });
        assert_eq!(rows[0].title, "notepda.txt");
    }

    #[test]
    fn strong_app_leads_over_files() {
        // "جوجل كر" starts an Arabic name of "Google Chrome", not its title.
        let items = catalog_items(&["Google Chrome"], "جوجل كر");
        let files = vec![Row::file(r"C:\جوجل كريم.txt", false)];
        let rows = arrange(Parts { query: "جوجل كر".into(), pins: vec![], items, files, files_done: true });
        assert_eq!(rows[0].title, "Google Chrome");
        // A word somewhere in an alias ("لقطة شاشة") is not enough: the file leads.
        let items = catalog_items(&["Snipping Tool"], "شاشة");
        assert!(items.iter().any(|i| i.title == "Snipping Tool"));
        let files = vec![Row::file(r"C:\شاشة.png", false)];
        let rows = arrange(Parts { query: "شاشة".into(), pins: vec![], items, files, files_done: true });
        assert_eq!(rows[0].title, "شاشة.png");
        // An Arabic name of the app.
        let items = catalog_items(&["Spotify"], "سبوتيفاي");
        let files = vec![Row::file(r"C:\سبوتيفاي.txt", false)];
        let rows = arrange(Parts { query: "سبوتيفاي".into(), pins: vec![], items, files, files_done: true });
        assert_eq!(rows[0].title, "Spotify");
    }

    #[test]
    fn a_sum_hides_files_that_only_share_digits() {
        let items = catalog_items(&["Notepad"], "2+2");
        assert_eq!(items.first().map(|i| i.kind), Some(Kind::Answer));
        let files = vec![Row::file(r"C:\x\report 2022.docx", false), Row::file(r"C:\2 2.txt", false), Row::file(r"C:\a\2+2 notes.txt", false)];
        let rows = arrange(Parts { query: "2+2".into(), pins: vec![], items, files, files_done: true });
        assert_eq!(rows[0].target, Target::Launcher(Action::CopyText("4".into())));
        let files: Vec<&str> = rows.iter().filter(|r| r.group == Group::Files).map(|r| r.title.as_str()).collect();
        assert_eq!(files, ["2+2 notes.txt"]);
        // Without an answer the files stay.
        let rows = arrange(Parts { query: "2".into(), pins: vec![], items: vec![], files: vec![Row::file(r"C:\2 2.txt", false)], files_done: true });
        assert_eq!(rows[0].title, "2 2.txt");
        assert!(names_query("Report 2024.PDF", "report ext:pdf !old") && !names_query("notepad.ini", "notepda"));
    }

    #[test]
    fn targets() {
        assert_eq!(Target::File { path: "C:\\a".into(), is_dir: false }.path(), Some("C:\\a"));
        assert_eq!(Target::Launcher(launch("C:\\x.lnk")).path(), Some("C:\\x.lnk"));
        assert_eq!(Target::Launcher(launch("shell:AppsFolder\\x")).path(), None);
        assert_eq!(Target::Launcher(launch("shell:AppsFolder\\x")).pin_target(), Some("shell:AppsFolder\\x"));
        assert_eq!(Target::Launcher(Action::OpenUri("ms-settings:".into())).pin_target(), None);
        let it = Item { icon_path: Some("C:\\x.exe".into()), ..item(Kind::App, "X", launch("C:\\x.exe")) };
        assert_eq!(Row::from_item(it).icon, Icon::Shell { path: "C:\\x.exe".into(), is_dir: false });
        assert_eq!(Row::from_item(item(Kind::Setting, "S", Action::OpenUri("u".into()))).icon, Icon::Glyph(glyph::SETTINGS));
        assert_eq!(Row::file("C:\\", true).title, "C:\\");
    }

    #[test]
    fn selection_moves() {
        let rows = vec![Row::file("C:\\a", false), Row::file("C:\\b", false)];
        assert_eq!(reselect(&rows, Some(&Target::File { path: "C:\\b".into(), is_dir: false })), Some(1));
        assert_eq!(reselect(&rows, Some(&Target::OpenInMain("x".into()))), Some(0));
        assert_eq!(reselect(&[], None), None);
        assert_eq!(move_selection(Some(0), 1, 3), Some(1));
        assert_eq!(move_selection(Some(2), 1, 3), Some(0));
        assert_eq!(move_selection(Some(0), -1, 3), Some(2));
        assert_eq!(move_selection(None, 1, 3), Some(0));
        assert_eq!(move_selection(None, -1, 3), Some(2));
        assert_eq!(move_selection(Some(1), 1, 0), None);
    }

    #[test]
    fn list_layout() {
        let m = ListMetrics { row: 40, more: 32, header: 24, separator: 9, pad: 6 };
        let g = [Group::Best, Group::Apps, Group::Apps, Group::More, Group::More];
        let (slots, h) = layout_list(&g, &m);
        let kinds: Vec<SlotKind> = slots.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SlotKind::Header(Group::Best),
                SlotKind::Row(0),
                SlotKind::Header(Group::Apps),
                SlotKind::Row(1),
                SlotKind::Row(2),
                SlotKind::Separator,
                SlotKind::Row(3),
                SlotKind::Row(4),
            ]
        );
        assert_eq!(h, 6 + 24 + 40 + 24 + 80 + 9 + 64 + 6);
        assert_eq!(row_at(&slots, 6 + 24 + 5), Some(0));
        assert_eq!(row_at(&slots, 6 + 2), None); // a header
        assert_eq!(row_at(&slots, h - 10), Some(4));
        assert_eq!(row_at(&slots, h + 10), None);
        // Only "more" rows: no separator, no header.
        let (slots, _) = layout_list(&[Group::More], &m);
        assert_eq!(slots.len(), 1);
        assert_eq!(layout_list(&[], &m).1, 0);
        // Too tall: result rows go, "more" rows stay.
        let mut rows: Vec<Row> = (0..10).map(|i| Row::file(&format!("C:\\{i}"), false)).collect();
        rows.push(Row { group: Group::More, ..Row::file("C:\\m", false) });
        fit_rows(&mut rows, &m, 300);
        let groups: Vec<Group> = rows.iter().map(|r| r.group).collect();
        assert!(layout_list(&groups, &m).1 <= 300);
        assert_eq!(rows.last().unwrap().group, Group::More);
        assert_eq!(rows[0].title, "0");
        fit_rows(&mut rows, &m, 1);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn pixels() {
        assert_eq!(premul(0xFF_80_00, 255), 0xFF_FF_80_00);
        assert_eq!(premul(0xFF_FF_FF, 128), 0x80_80_80_80);
        assert_eq!(premul(0x12_34_56, 0), 0);
        // Opaque on top wins; transparent on top changes nothing.
        assert_eq!(over(0xFF_10_20_30, 0xFF_FF_FF_FF), 0xFF_10_20_30);
        assert_eq!(over(0, 0x80_40_40_40), 0x80_40_40_40);
        // Half white over opaque black: mid grey.
        assert_eq!(over(premul(0xFF_FF_FF, 128), 0xFF_00_00_00), 0xFF_80_80_80);
        assert_eq!(fade(0xFF_FF_FF_FF, 0), 0);
        assert_eq!(fade(0xFF_FF_FF_FF, 255), 0xFF_FF_FF_FF);
        // Matting: untouched background is transparent, solid paint opaque,
        // antialiased edges in between.
        assert_eq!(matte(0xFF_00_00_00, 0xFF_FF_FF_FF), 0);
        assert_eq!(matte(0xFF_12_34_56, 0xFF_12_34_56), 0xFF_12_34_56);
        let half = matte(0x00_40_40_40, 0x00_BF_BF_BF);
        assert_eq!(half >> 24, 128);
        assert_eq!(half & 0xFF, 0x40);
        // Premultiplied channels never exceed alpha, whatever the input.
        for (b, w) in [(0xFF_FF_00_00, 0xFF_FF_10_10), (0x00_FF_FF_FF, 0x00_00_00_00), (0x00_30_90_10, 0x00_F0_A0_80)] {
            let m = matte(b, w);
            let a = m >> 24;
            assert!((m & 0xFF) <= a && ((m >> 8) & 0xFF) <= a && ((m >> 16) & 0xFF) <= a, "{m:#x}");
        }
    }

    #[test]
    fn rounded_shapes_and_icon() {
        // Centre covered, corners cut, straight edges full.
        assert_eq!(rounded_coverage(50, 18, 100, 36, 18.0), 255);
        assert_eq!(rounded_coverage(0, 0, 100, 36, 18.0), 0);
        assert_eq!(rounded_coverage(50, 0, 100, 36, 18.0), 255);
        let edge = rounded_coverage(4, 5, 100, 36, 18.0);
        assert!(edge > 0 && edge < 255, "{edge}");
        assert_eq!(rounded_coverage(0, 0, 10, 10, 0.0), 255);
        for size in [16, 20, 24, 32, 48] {
            let px = magnifier_icon(size, 0xFF_FF_FF);
            assert_eq!(px.len(), (size * size) as usize);
            // Something is drawn, the lens centre is hollow, the corners empty.
            let at = |x: i32, y: i32| px[(y * size + x) as usize] >> 24;
            assert!(px.iter().filter(|p| **p >> 24 > 200).count() > (size * size / 12) as usize, "{size}");
            let c = (size as f32 * 0.42) as i32;
            assert_eq!(at(c, c), 0, "{size}");
            assert_eq!(at(0, size - 1), 0);
            assert_eq!(at(size - 1, 0), 0);
        }
        assert_eq!(magnifier_icon(0, 0).len(), 1);
        // Dark and light palettes differ where it matters.
        assert_ne!(Palette::new(true).text, Palette::new(false).text);
        assert!(has_backdrop(22621) && !has_backdrop(22000) && has_round_corners(22000) && !has_round_corners(19045));
    }
}
