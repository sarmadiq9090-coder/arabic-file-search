//! Which windows are Open / Save As dialogs, for Ctrl+Alt+G (`ui/quickswitch.rs`).
//!
//! Recognised from a description of the window and its child windows, so
//! it is unit-tested on Linux. Plenty of other dialogs share the dialog
//! class (`#32770`) and even the control id of the old file-name box
//! (edt1): Notepad's Find ("Find what"), Replace, Print (the page range).
//! Typing a folder there and pressing OK would do something else entirely,
//! so a file dialog needs a file-name box AND a view of the files:
//! - the common item dialog (Vista and later) and the Explorer-style old
//!   one: the file-name combo box (cmb13) with an edit in it, and the
//!   shell's file view (`SHELLDLL_DefView`) or the item dialog's frame
//!   (`DUIViewWndClassName`, also there when a Save dialog's folders are
//!   hidden);
//! - the oldest dialog: the file-name edit (edt1) and the file list box (lst1).

/// Control id of the file-name combo box (cmb13).
pub const ID_FILENAME_COMBO: i32 = 0x47C;
/// Control id of the oldest dialogs' file-name edit (edt1).
pub const ID_FILENAME_EDIT: i32 = 0x480;
/// Control id of the oldest dialogs' file list box (lst1).
const ID_FILE_LIST: i32 = 0x460;

/// A window as far as recognising it goes: its class, its control id and
/// its child windows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub class: String,
    pub id: i32,
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

/// Whether `root` is an Open / Save As dialog that Ctrl+Alt+G can move.
pub fn is_file_dialog(root: &Node) -> bool {
    if root.class != "#32770" {
        return false;
    }
    let combo_box = root.descendants().any(|n| n.id == ID_FILENAME_COMBO && (n.is("Edit") || n.descendants().any(|c| c.is("Edit"))));
    let file_view = root.descendants().any(|n| n.is("SHELLDLL_DefView") || n.is("DUIViewWndClassName"));
    let oldest = root.descendants().any(|n| n.id == ID_FILENAME_EDIT && n.is("Edit"))
        && root.descendants().any(|n| n.id == ID_FILE_LIST && n.is("ListBox"));
    (combo_box && file_view) || oldest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(class: &str, id: i32, children: Vec<Node>) -> Node {
        Node { class: class.into(), id, children }
    }

    fn leaf(class: &str, id: i32) -> Node {
        n(class, id, Vec::new())
    }

    fn buttons() -> Vec<Node> {
        vec![leaf("Button", 1), leaf("Button", 2)]
    }

    /// Windows 10/11 Open dialog (common item dialog), as Spy++ shows it.
    fn item_dialog_open() -> Node {
        let mut kids = vec![
            n("DUIViewWndClassName", 0, vec![n("DirectUIHWND", 0, vec![
                n("CtrlNotifySink", 0, vec![n("SHELLDLL_DefView", 0, vec![n("DirectUIHWND", 0, vec![])])]),
                n("CtrlNotifySink", 0, vec![leaf("NamespaceTreeControl", 0)]),
            ])]),
            n("WorkerW", 0, vec![n("ReBarWindow32", 0, vec![leaf("Address Band Root", 0)])]),
            n("ComboBoxEx32", ID_FILENAME_COMBO, vec![n("ComboBox", 0, vec![leaf("Edit", 0)])]),
            leaf("ComboBox", 0x470),
        ];
        kids.extend(buttons());
        n("#32770", 0, kids)
    }

    #[test]
    fn file_dialogs_are_recognised() {
        assert!(is_file_dialog(&item_dialog_open()));
        // Save As with the folders hidden: no file view window, but the item dialog's frame.
        let save_collapsed = n("#32770", 0, vec![
            n("DUIViewWndClassName", 0, vec![leaf("DirectUIHWND", 0)]),
            n("ComboBox", ID_FILENAME_COMBO, vec![leaf("Edit", ID_FILENAME_COMBO)]),
            leaf("Button", 1),
        ]);
        assert!(is_file_dialog(&save_collapsed));
        // The Explorer-style old dialog (GetOpenFileName with a hook or template).
        let explorer_style = n("#32770", 0, vec![
            n("SHELLDLL_DefView", 0x461, vec![leaf("SysListView32", 1)]),
            leaf("Static", 0x442),
            n("ComboBox", ID_FILENAME_COMBO, vec![leaf("Edit", 0x3E9)]),
            leaf("Button", 1),
        ]);
        assert!(is_file_dialog(&explorer_style));
        // The oldest (Windows 3.1 style) dialog.
        let oldest = n("#32770", 0, vec![
            leaf("Edit", ID_FILENAME_EDIT),
            leaf("ListBox", ID_FILE_LIST),
            leaf("ListBox", 0x461),
            leaf("ComboBox", 0x470),
            leaf("ComboBox", 0x471),
            leaf("Button", 1),
        ]);
        assert!(is_file_dialog(&oldest));
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
        // Not a dialog window at all, even with the right children.
        let mut not_dialog = item_dialog_open();
        not_dialog.class = "CabinetWClass".into();
        assert!(!is_file_dialog(&not_dialog));
        assert!(!is_file_dialog(&Node::default()));
    }

}
