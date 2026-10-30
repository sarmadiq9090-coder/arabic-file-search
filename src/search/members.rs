//! Content hits inside archives. The archive is one result row; each member
//! is matched on its own (every term in the same member), and the snippet
//! and matching lines name the member they come from:
//! `تقارير\تقرير.docx: …نص…`.

use super::{ContentHit, MAX_LINES, Query, content_hit};

/// One hit for the archive at entry `index` from its `(member, text)`
/// pairs: the snippet of the first matching member, the matches of all of
/// them counted, and up to [`MAX_LINES`] lines, each prefixed with its
/// member. None when no member matches.
pub(super) fn archive_hit(index: u32, members: &[(&str, &str)], q: &Query) -> Option<ContentHit> {
    let mut acc: Option<ContentHit> = None;
    for (name, text) in members {
        let Some(h) = content_hit(index, text, q) else { continue };
        let acc = acc.get_or_insert_with(|| ContentHit {
            index,
            snippet: format!("{name}: {}", h.snippet),
            count: 0,
            lines: Vec::new(),
        });
        acc.count = acc.count.saturating_add(h.count);
        let room = MAX_LINES.saturating_sub(acc.lines.len());
        acc.lines.extend(h.lines.into_iter().take(room).map(|(n, s)| (n, format!("{name}: {s}"))));
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Query {
        Query::parse_at(s, 1_700_000_000, 0)
    }

    #[test]
    fn member_prefixed_hit() {
        let members = [
            ("صور\\a.txt", "لا شيء هنا"),
            ("تقارير\\تقرير.docx", "مقدمة\nارتفعت المبيعات هذا العام\nختام المبيعات"),
            ("ملاحظات.txt", "المبيعات"),
        ];
        let h = archive_hit(3, &members, &q("المبيعات")).unwrap();
        assert_eq!(h.index, 3);
        assert_eq!(h.snippet, "تقارير\\تقرير.docx: مقدمة ارتفعت المبيعات هذا العام ختام المبيعات");
        assert_eq!(h.count, 3);
        assert_eq!(
            h.lines,
            [
                (2, "تقارير\\تقرير.docx: ارتفعت المبيعات هذا العام".to_string()),
                (3, "تقارير\\تقرير.docx: ختام المبيعات".to_string()),
                (1, "ملاحظات.txt: المبيعات".to_string()),
            ]
        );
        assert!(archive_hit(0, &members, &q("مفقود")).is_none());
        // Every term in the same member.
        assert!(archive_hit(0, &members, &q("شيء ختام")).is_none());
    }

    #[test]
    fn content_hit_splits_archive_text() {
        // The text extract_text gives an archive (see extract::archive).
        let dir = std::env::temp_dir().join(format!("bahth_members_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("أرشيف.zip");
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        zw.start_file("تقارير/مبيعات.txt", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zw, "تقرير المبيعات الشهري".as_bytes()).unwrap();
        zw.finish().unwrap();
        let text = crate::extract::extract_text(&p).unwrap();
        let h = content_hit(5, &text, &q("المبيعات")).unwrap();
        assert_eq!(h.snippet, "تقارير\\مبيعات.txt: تقرير المبيعات الشهري");
        assert_eq!(h.count, 1);
        assert_eq!(h.lines, [(1, "تقارير\\مبيعات.txt: تقرير المبيعات الشهري".to_string())]);
        // Member names are not content.
        assert!(content_hit(5, &text, &q("تقارير")).is_none());
    }
}
