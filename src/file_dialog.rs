//! Which windows are Open / Save As dialogs, and which of their windows is
//! the file-name box, for Ctrl+Alt+G (`ui/quickswitch.rs`).
//!
//! Recognised from a description of the window and its child windows, so
//! it is unit-tested on Linux. Plenty of other dialogs share the dialog
//! class (`#32770`) and even the control id of the old file-name box
//! (edt1): Notepad's Find ("Find what"), Replace, Print (the page range).
//! Typing a folder there and pressing OK would do something else entirely,
//! so a file dialog needs a file-name box AND a view of the files:
//! - the common item dialog (Vista and later) and the Explorer-style old
//!   one: the shell's file view (`SHELLDLL_DefView`) or the item dialog's
//!   frame (`DUIViewWndClassName`, also there when a Save dialog's folders
//!   are hidden), and a file-name box, which is either
//!   - the file-name combo box (cmb13) with an edit in it (Open dialogs,
//!     the Explorer-style old dialogs), or
//!   - an editable combo box the item dialog floats over its frame
//!     (`FloatNotifySink > ComboBox > Edit`, the edit's id 0x3E9): the Save
//!     dialog on Windows 11 25H2 has only this one, with no cmb13 at all;
//! - the oldest dialog: the file-name edit (edt1) and the file list box (lst1).

/// Control id of the file-name combo box (cmb13).
pub const ID_FILENAME_COMBO: i32 = 0x47C;
/// Control id of the oldest dialogs' file-name edit (edt1).
pub const ID_FILENAME_EDIT: i32 = 0x480;
/// Control id of the oldest dialogs' file list box (lst1).
const ID_FILE_LIST: i32 = 0x460;
/// Control id of the edit inside a file-name combo box (1001; UI
/// Automation shows it as AutomationId "1001").
pub const ID_COMBO_EDIT: i32 = 0x3E9;

/// A window as far as recognising it goes: its class, its control id,
/// whether it is hidden, its child windows, and its handle (as a number;
/// only the caller reads it, 0 in tests that don't care).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub class: String,
    pub id: i32,
    pub hidden: bool,
    pub handle: usize,
    pub children: Vec<Node>,
}

impl Node {
    /// Every window below this one, depth first.
    fn descendants(&self) -> impl Iterator<Item = &Node> {
        let mut stack: Vec<&Node> = self.children.iter().rev().collect();
        std::iter::from_fn(move || {
            let n = stack.pop()?;
            stack.extend(n.children.iter().rev());
            Some(n)
        })
    }

    fn is(&self, class: &str) -> bool {
        self.class.eq_ignore_ascii_case(class)
    }
}

/// A file-name box candidate and whether it (or a window holding it) is hidden.
type Candidate<'a> = (&'a Node, bool);

/// The edit of the file-name combo box (cmb13): the combo itself when it is
/// an edit, or the first edit inside it.
fn combo_edits<'a>(root: &'a Node, out: &mut Vec<Candidate<'a>>) {
    fn walk<'a>(n: &'a Node, hidden: bool, out: &mut Vec<Candidate<'a>>) {
        let hidden = hidden || n.hidden;
        if n.id == ID_FILENAME_COMBO {
            if n.is("Edit") {
                out.push((n, hidden));
                return;
            }
            if let Some(e) = first_edit(n, hidden) {
                out.push(e);
                return;
            }
        }
        for c in &n.children {
            walk(c, hidden, out);
        }
    }
    walk(root, false, out);
}

/// The first edit below `n`, with whether it is hidden.
fn first_edit(n: &Node, hidden: bool) -> Option<Candidate<'_>> {
    for c in &n.children {
        let h = hidden || c.hidden;
        if c.is("Edit") {
            return Some((c, h));
        }
        if let Some(e) = first_edit(c, h) {
            return Some(e);
        }
    }
    None
}

/// The edits of the editable combo boxes the item dialog floats over its
/// frame (`FloatNotifySink > ComboBox > Edit`), the file-name one (0x3E9)
/// first. The "Save as type" box is a drop-down list there: no edit. The
/// address bar's edit sits under `Address Band Root`, never in a sink.
fn floating_edits<'a>(root: &'a Node, out: &mut Vec<Candidate<'a>>) {
    fn walk<'a>(n: &'a Node, hidden: bool, in_sink: bool, found: &mut Vec<Candidate<'a>>) {
        let hidden = hidden || n.hidden;
        let in_sink = in_sink || n.is("FloatNotifySink");
        if in_sink && n.is("ComboBox") {
            if let Some(e) = first_edit(n, hidden) {
                found.push(e);
            }
            return;
        }
        for c in &n.children {
            walk(c, hidden, in_sink, found);
        }
    }
    let mut found = Vec::new();
    walk(root, false, false, &mut found);
    // Stable: the file-name edit first, the others in window order.
    found.sort_by_key(|(e, _)| e.id != ID_COMBO_EDIT);
    out.extend(found);
}

/// The window to type a folder into, when `root` is an Open / Save As
/// dialog that Ctrl+Alt+G can move; `None` for any other window.
pub fn filename_box(root: &Node) -> Option<&Node> {
    if root.class != "#32770" {
        return None;
    }
    let file_view = root.descendants().any(|n| n.is("SHELLDLL_DefView") || n.is("DUIViewWndClassName"));
    if file_view {
        let mut found = Vec::new();
        combo_edits(root, &mut found);
        floating_edits(root, &mut found);
        // A box on screen beats one that is hidden (an unused layout).
        if let Some(&(e, _)) = found.iter().find(|(_, hidden)| !hidden).or(found.first()) {
            return Some(e);
        }
    }
    let list = root.descendants().any(|n| n.id == ID_FILE_LIST && n.is("ListBox"));
    root.descendants().find(|n| n.id == ID_FILENAME_EDIT && n.is("Edit")).filter(|_| list)
}

/// Whether `root` is an Open / Save As dialog that Ctrl+Alt+G can move.
pub fn is_file_dialog(root: &Node) -> bool {
    filename_box(root).is_some()
}

/// What to do with the file-name box after the folder was typed into it
/// and OK pressed (`ui/quickswitch.rs`): the user's file name must come
/// back, as a Save As keeps the name it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameBox {
    /// The box shows the user's name: nothing to do (yet).
    Fine,
    /// Put the user's name back.
    Restore,
    /// Leave the box alone from now on: the user is typing in it.
    Stop,
}

/// The box shows `now`, the user had `original`, we typed `typed`, and
/// `early` says the click was moments ago (the user can't have typed yet).
/// Early on anything other than the user's name is the dialog's doing;
/// later only an empty box or the folder we typed is (a slow navigation
/// that finished late); anything else the user typed.
pub fn after_switch(now: &str, original: &str, typed: &str, early: bool) -> NameBox {
    if now == original {
        return NameBox::Fine;
    }
    let ours = now.trim().is_empty() || now.trim_end_matches(['\\', '/']) == typed.trim_end_matches(['\\', '/']);
    if early || ours { NameBox::Restore } else { NameBox::Stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(class: &str, id: i32, children: Vec<Node>) -> Node {
        Node { class: class.into(), id, children, ..Node::default() }
    }

    fn leaf(class: &str, id: i32) -> Node {
        n(class, id, Vec::new())
    }

    /// The file-name box of the fixtures: handle 7.
    fn name_edit(id: i32) -> Node {
        Node { handle: 7, ..leaf("Edit", id) }
    }

    fn hidden(mut node: Node) -> Node {
        node.hidden = true;
        node
    }

    fn buttons() -> Vec<Node> {
        vec![leaf("Button", 1), leaf("Button", 2)]
    }

    fn handle_of(root: &Node) -> Option<usize> {
        filename_box(root).map(|b| b.handle)
    }

    /// Windows 10/11 Open dialog (common item dialog), as Spy++ shows it.
    fn item_dialog_open() -> Node {
        let mut kids = vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("CtrlNotifySink", 0, vec![n("SHELLDLL_DefView", 0, vec![n("DirectUIHWND", 0, vec![])])]),
                n("CtrlNotifySink", 0, vec![leaf("NamespaceTreeControl", 0)]),
            ])]),
            n("WorkerW", 0, vec![n("ReBarWindow32", 0, vec![leaf("Address Band Root", 0)])]),
            n("ComboBoxEx32", ID_FILENAME_COMBO, vec![n("ComboBox", ID_FILENAME_COMBO, vec![name_edit(ID_FILENAME_COMBO)])]),
            leaf("ComboBox", 0x470),
        ];
        kids.extend(buttons());
        n("#32770", 0, kids)
    }

    /// The address bar while it is being edited: a combo with an edit, in
    /// the toolbar, never a file-name box.
    fn address_bar() -> Node {
        n("WorkerW", 0, vec![n("ReBarWindow32", 0, vec![n("Address Band Root", 0, vec![n("msctls_progress32", 0, vec![
            leaf("Breadcrumb Parent", 0),
            hidden(n("ComboBoxEx32", 0xA205, vec![n("ComboBox", 0xA205, vec![leaf("Edit", 0xA205)])])),
        ])])])])
    }

    /// Windows 11 25H2 Save As (IFileSaveDialog, WinForms SaveFileDialog,
    /// Notepad), as the window-tree dump of the FIN1 test machine shows it:
    /// no cmb13 and no edt1; the file name is in a combo box floated over
    /// the frame, the type in another one without an edit.
    fn item_dialog_save_25h2() -> Node {
        let mut kids = vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("CtrlNotifySink", 0, vec![n("SHELLDLL_DefView", 0, vec![n("DirectUIHWND", 0, vec![])])]),
                n("CtrlNotifySink", 0, vec![leaf("NamespaceTreeControl", 0)]),
                n("CtrlNotifySink", 0, vec![leaf("ScrollBar", 0)]),
                n("FloatNotifySink", 0, vec![n("ComboBox", 0, vec![name_edit(ID_COMBO_EDIT)])]),
                n("FloatNotifySink", 0, vec![leaf("ComboBox", 0)]),
            ])]),
            address_bar(),
        ];
        kids.extend(buttons());
        n("#32770", 0, kids)
    }

    #[test]
    fn file_dialogs_are_recognised() {
        assert_eq!(handle_of(&item_dialog_open()), Some(7));
        // Windows 11 25H2 Save As: the floating file-name combo (F1).
        assert_eq!(handle_of(&item_dialog_save_25h2()), Some(7));
        // Save As with the folders hidden (25H2): no file view window, but
        // the item dialog's frame and the same floating file-name combo.
        let save_collapsed = n("#32770", 0, vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("FloatNotifySink", 0, vec![n("ComboBox", 0, vec![name_edit(ID_COMBO_EDIT)])]),
                n("FloatNotifySink", 0, vec![leaf("ComboBox", 0)]),
            ])]),
            leaf("Button", 1),
            leaf("Button", 2),
        ]);
        assert_eq!(handle_of(&save_collapsed), Some(7));
        // Older Save As layouts: the file-name combo is cmb13 (in the sink or not).
        let save_cmb13 = n("#32770", 0, vec![
            n("DUIViewWndClassName", 0, vec![leaf("DirectUIHWND", 0)]),
            n("ComboBox", ID_FILENAME_COMBO, vec![name_edit(ID_FILENAME_COMBO)]),
            leaf("Button", 1),
        ]);
        assert_eq!(handle_of(&save_cmb13), Some(7));
        let sink_cmb13 = n("#32770", 0, vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("FloatNotifySink", 0, vec![n("ComboBox", ID_FILENAME_COMBO, vec![name_edit(ID_COMBO_EDIT)])]),
            ])]),
            leaf("Button", 1),
        ]);
        assert_eq!(handle_of(&sink_cmb13), Some(7));
        // The Explorer-style old dialog (GetOpenFileName with a hook or template).
        let explorer_style = n("#32770", 0, vec![
            n("SHELLDLL_DefView", 0x461, vec![leaf("SysListView32", 1)]),
            leaf("Static", 0x442),
            n("ComboBox", ID_FILENAME_COMBO, vec![name_edit(ID_COMBO_EDIT)]),
            leaf("Button", 1),
        ]);
        assert_eq!(handle_of(&explorer_style), Some(7));
        // The oldest (Windows 3.1 style) dialog.
        let oldest = n("#32770", 0, vec![
            name_edit(ID_FILENAME_EDIT),
            leaf("ListBox", ID_FILE_LIST),
            leaf("ListBox", 0x461),
            leaf("ComboBox", 0x470),
            leaf("ComboBox", 0x471),
            leaf("Button", 1),
        ]);
        assert_eq!(handle_of(&oldest), Some(7));
        for d in [item_dialog_open(), item_dialog_save_25h2(), save_collapsed, save_cmb13, explorer_style, oldest] {
            assert!(is_file_dialog(&d), "{d:?}");
        }
    }

    #[test]
    fn the_name_box_on_screen_wins() {
        // A hidden cmb13 left over from another layout, the floating box shown.
        let mut d = item_dialog_save_25h2();
        d.children.push(hidden(n("ComboBoxEx32", ID_FILENAME_COMBO, vec![n("ComboBox", 0, vec![leaf("Edit", 0)])])));
        assert_eq!(handle_of(&d), Some(7));
        // Another editable floating combo before the file-name one: 0x3E9 wins.
        let d = n("#32770", 0, vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("FloatNotifySink", 0, vec![n("ComboBox", 0, vec![leaf("Edit", 0x3EA)])]),
                n("FloatNotifySink", 0, vec![n("ComboBox", 0, vec![name_edit(ID_COMBO_EDIT)])]),
            ])]),
            leaf("Button", 1),
        ]);
        assert_eq!(handle_of(&d), Some(7));
        // Everything hidden: still the file-name box rather than nothing.
        let mut d = item_dialog_save_25h2();
        d.children[0].hidden = true;
        assert_eq!(handle_of(&d), Some(7));
    }

    #[test]
    fn other_dialogs_are_not() {
        // Notepad's Find: "Find what" is edt1, as in the oldest file dialog.
        let find = n("#32770", 0, vec![
            leaf("Static", 0xFFFF),
            leaf("Edit", ID_FILENAME_EDIT),
            leaf("Button", 0x410),
            leaf("Button", 0x411),
            leaf("Button", 1),
            leaf("Button", 2),
        ]);
        assert!(!is_file_dialog(&find));
        let replace = n("#32770", 0, vec![leaf("Edit", ID_FILENAME_EDIT), leaf("Edit", 0x481), leaf("Button", 1)]);
        assert!(!is_file_dialog(&replace));
        // The classic Print dialog: page range in edt1/edt2, printers in a combo.
        let print = n("#32770", 0, vec![leaf("ComboBox", 0x473), leaf("Edit", ID_FILENAME_EDIT), leaf("Edit", 0x481), leaf("Edit", 0x482), leaf("Button", 1)]);
        assert!(!is_file_dialog(&print));
        // The newer Print dialog: printers in a shell view, the page range in edt1, tabs.
        let print_ex = n("#32770", 0, vec![
            leaf("SysTabControl32", 0x3020),
            n("#32770", 0, vec![n("SHELLDLL_DefView", 0, vec![leaf("SysListView32", 0)]), leaf("Edit", ID_FILENAME_EDIT), leaf("Button", 0x420)]),
            leaf("Button", 1),
        ]);
        assert!(!is_file_dialog(&print_ex));
        // A task dialog or another DirectUI dialog: no file-name box.
        assert!(!is_file_dialog(&n("#32770", 0, vec![leaf("DirectUIHWND", 0)])));
        // The file-name combo without an edit (a drop-down list) or without a view.
        assert!(!is_file_dialog(&n("#32770", 0, vec![leaf("ComboBox", ID_FILENAME_COMBO), leaf("SHELLDLL_DefView", 0)])));
        assert!(!is_file_dialog(&n("#32770", 0, vec![n("ComboBox", ID_FILENAME_COMBO, vec![leaf("Edit", 0)])])));
        // An item dialog frame with only drop-down lists floating, and the
        // address bar's edit: nothing to type a folder into.
        let no_name = n("#32770", 0, vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("FloatNotifySink", 0, vec![leaf("ComboBox", 0)]),
                n("CtrlNotifySink", 0, vec![n("ComboBox", 0, vec![leaf("Edit", ID_COMBO_EDIT)])]),
            ])]),
            address_bar(),
            leaf("Button", 1),
        ]);
        assert!(!is_file_dialog(&no_name));
        // A floating editable combo without the item dialog's frame or a view.
        let no_view = n("#32770", 0, vec![n("DirectUIHWND", 0, vec![n("FloatNotifySink", 0, vec![n("ComboBox", 0, vec![leaf("Edit", ID_COMBO_EDIT)])])]), leaf("Button", 1)]);
        assert!(!is_file_dialog(&no_view));
        // Not a dialog window at all, even with the right children.
        for mut not_dialog in [item_dialog_open(), item_dialog_save_25h2()] {
            not_dialog.class = "CabinetWClass".into();
            assert!(!is_file_dialog(&not_dialog));
        }
        assert!(!is_file_dialog(&Node::default()));
    }

    #[test]
    fn the_users_file_name_comes_back() {
        let typed = r"C:\Users\سيف\Documents\";
        // The box still shows the name: nothing to do.
        assert_eq!(after_switch("تقرير.txt", "تقرير.txt", typed, true), NameBox::Fine);
        assert_eq!(after_switch("", "", typed, false), NameBox::Fine);
        // The dialog left our folder in the box, or emptied it: the name comes back, early or late.
        for early in [true, false] {
            assert_eq!(after_switch(typed, "تقرير.txt", typed, early), NameBox::Restore);
            assert_eq!(after_switch(r"C:\Users\سيف\Documents", "تقرير.txt", typed, early), NameBox::Restore);
            assert_eq!(after_switch("", "تقرير.txt", typed, early), NameBox::Restore);
        }
        // Early, anything else is the dialog rewriting the box too.
        assert_eq!(after_switch("Documents", "تقرير.txt", typed, true), NameBox::Restore);
        // Later, other text is the user typing: left alone.
        assert_eq!(after_switch("تقرير-2", "تقرير.txt", typed, false), NameBox::Stop);
    }
}
