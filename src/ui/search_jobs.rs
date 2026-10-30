//! Loading and scanning the index, and the filename and document searches,
//! all running on background threads that the timer polls.

use native_windows_gui as nwg;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::content_index;
use super::files::flush_index_save;
use super::format::fmt_num;
use super::startup::{LOOP_STARTED, background_priority, log, log_phases, phase};
use super::{App, ContentJob, LoadJob, Matches, Row, ScanJob};
use crate::index::{self, Index, ScanProgress};
use crate::fuzzy;
use crate::search::{self, ContentHit, ContentProgress, Query, SortKey};

/// The outcome of a background filename search.
pub(super) struct NameResult {
    index: Arc<Index>,
    hits: Vec<u32>,
    text: String,
    /// How a fallback (keyboard layout, Arabizi, typos) found the hits, if one did.
    converted: Option<fuzzy::Fallback>,
    empty: bool,
    ms: usize,
}

impl App {
    // ---------------------------------------------------------------- index

    /// Reads the cached index on a background thread so the window appears at once.
    pub(super) fn start(&self) {
        let slot: LoadJob = Arc::new(Mutex::new(None));
        let s = slot.clone();
        std::thread::spawn(move || {
            background_priority();
            let started = std::time::Instant::now();
            let idx = Index::load(&index::cache_path());
            log(&format!(
                "index load: {} entries in {} ms",
                idx.as_ref().map_or(0, |i| i.entries.len()),
                started.elapsed().as_millis()
            ));
            if let Ok(mut slot) = s.lock() {
                *slot = Some(idx);
            }
        });
        self.state.borrow_mut().loading = Some(slot);
        self.set_status(tr!("جاري تحميل الفهرس…", "Loading the index…"));
        self.query.set_focus();
    }

    pub(super) fn poll_load(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.loading else { return };
        let Some(loaded) = job.lock().ok().and_then(|mut r| r.take()) else { return };
        st.loading = None;
        match loaded {
            Some(idx) => {
                let n = idx.entries.len();
                st.index = Arc::new(idx);
                drop(st);
                self.start_warm();
                self.start_watching();
                if self.query.text().trim().is_empty() || self.content_mode() {
                    self.set_status(&trf!("تم تحميل الفهرس ({} عنصر). اكتب للبحث.", "Index loaded ({} items). Type to search.", fmt_num(n)));
                } else {
                    self.search_names();
                }
            }
            None => {
                drop(st);
                self.start_scan();
            }
        }
    }

    pub(super) fn start_scan(&self) {
        // An edit still waiting to be saved must not land after the new scan.
        flush_index_save();
        // The scan sees every change; watching resumes when it is done.
        self.stop_watching();
        let mut st = self.state.borrow_mut();
        if st.loading.is_some() {
            drop(st);
            self.set_status(tr!("انتظر حتى يكتمل تحميل الفهرس، ثم حدّثه.", "Wait until the index has loaded, then update it."));
            return;
        }
        if st.scan.is_some() {
            return;
        }
        let progress = Arc::new(ScanProgress::default());
        let result = Arc::new(Mutex::new(None));
        let (p, r) = (progress.clone(), result.clone());
        std::thread::spawn(move || {
            background_priority();
            let started = std::time::Instant::now();
            let idx = Index::build(&index::list_drives(), &p);
            log(&format!("index build: {} entries in {} ms", idx.entries.len(), started.elapsed().as_millis()));
            let _ = idx.save(&index::cache_path());
            if let Ok(mut slot) = r.lock() {
                *slot = Some(idx);
            }
        });
        st.scan = Some(ScanJob { progress, result });
        drop(st);
        self.reindex_btn.set_enabled(false);
        self.set_status(tr!("جاري فهرسة الملفات على كل الأقراص…", "Indexing files on all drives…"));
    }

    // ---------------------------------------------------------------- search

    pub(super) fn content_mode(&self) -> bool {
        self.content_chk.check_state() == nwg::CheckBoxState::Checked
    }

    pub(super) fn kind(&self) -> usize {
        self.kind_combo.selection().unwrap_or(0)
    }

    pub(super) fn parse_query(&self, text: &str) -> Query {
        let mut q = Query::parse(text);
        q.apply_kind(self.kind());
        q
    }

    /// Live filename search, run on every keystroke.
    pub(super) fn on_query_changed(&self) {
        if !self.content_mode() {
            self.search_names();
        }
    }

    pub(super) fn on_search(&self) {
        if self.content_mode() {
            self.search_content();
        } else {
            self.search_names();
        }
    }

    pub(super) fn on_kind_changed(&self) {
        self.state.borrow_mut().settings.kind = self.kind();
        if !self.content_mode() {
            self.search_names();
        }
    }

    /// Starts a filename search on a background thread, so typing never
    /// waits for a search over millions of names. While one runs, later
    /// keystrokes are folded into a single follow-up search.
    pub(super) fn search_names(&self) {
        self.cancel_content();
        let mut st = self.state.borrow_mut();
        if st.name_running {
            st.name_pending = true;
            return;
        }
        st.name_running = true;
        st.name_pending = false;
        let index = st.index.clone();
        let scope = st.scope.clone();
        let (sort, desc) = (SortKey::from_column(st.settings.sort_col.max(-1) as usize), st.settings.sort_desc);
        let sort = if st.settings.sort_col < 0 { SortKey::Relevance } else { sort };
        let boosted = if sort == SortKey::Relevance { st.history.boosted(&index, index::now_secs()) } else { Vec::new() };
        drop(st);

        let text = self.query.text();
        let kind = self.kind();
        let parse = move |t: &str| {
            let mut q = Query::parse(t);
            q.apply_kind(kind);
            q
        };
        let slot = self.name_result.clone();
        let notice = self.name_notice.sender();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let q = parse(&text);
            let mut hits = search::search_names(&index.entries, &q, &scope);
            // Wrong keyboard layout ("hgjrvdv" → "التقرير"), Arabizi ("3abd"), then typos.
            let mut converted = None;
            if hits.is_empty()
                && let Some((alt_hits, fb)) = fuzzy::name_fallback(&index.entries, &text, &scope, &parse)
            {
                hits = alt_hits;
                converted = Some(fb);
            }
            if sort == SortKey::Relevance {
                // Files the user opens often come first.
                search::promote(&mut hits, &boosted);
            } else if sort == SortKey::Size {
                // Folders sort by everything inside them.
                crate::folder_sizes::sort_hits_by_total(&mut hits, &crate::folder_sizes::for_index(&index), desc);
            } else {
                search::sort_hits(&index.entries, &mut hits, sort, desc);
            }
            let ms = started.elapsed().as_millis() as usize;
            if let Ok(mut r) = slot.lock() {
                *r = Some(NameResult { index, hits, text, converted, empty: q.is_empty(), ms });
            }
            notice.notice();
        });
    }

    pub(super) fn on_names_found(&self) {
        let Some(res) = self.name_result.lock().ok().and_then(|mut r| r.take()) else { return };
        let selected = self.selected_path();
        let mut st = self.state.borrow_mut();
        st.name_running = false;
        let live = std::mem::take(&mut st.live_search);
        if st.content.is_some() || self.content_mode() {
            // A document search took over meanwhile; leave it alone.
            st.name_pending = false;
            return;
        }
        if st.name_pending {
            // The query changed meanwhile: these results are already stale.
            drop(st);
            self.search_names();
            return;
        }
        let old_n = st.rows.len();
        st.rows = res.hits.into_iter().map(|entry| Row { entry, snippet: None, matches: None }).collect();
        st.rows_index = res.index;
        st.shown_query = Some(self.parse_query(res.converted.as_ref().map_or(&res.text, |f| &f.query)));
        let n = st.rows.len();
        let busy = st.scan.is_some() || st.loading.is_some();
        let loading = st.loading.is_some();
        let warn = warnings_text(st.shown_query.as_ref());
        drop(st);
        if live {
            // Files changed on disk: keep the selection, and say only that
            // the count changed.
            self.refresh_list();
            self.reselect(selected);
            if n != old_n && !res.empty {
                self.set_status(&trf!("وجدت {} نتيجة (حُدِّثت تلقائياً){warn}", "Found {} results (updated automatically){warn}", fmt_num(n)));
            }
            return;
        }
        self.clear_selection();
        self.refresh_list();
        if res.empty {
            if !busy {
                self.set_status(tr!("اكتب اسم الملف أو جزءاً منه للبحث.", "Type a file name or part of one to search."));
            }
            return;
        }
        let tail = match (loading, busy) {
            (true, _) => tr!(" (جاري تحميل الفهرس…)", " (loading the index…)"),
            (false, true) => tr!(" (الفهرسة ما زالت جارية)", " (indexing is still running)"),
            _ => "",
        };
        let tail = format!("{tail}{warn}");
        match res.converted {
            Some(fb) => self.set_status(&trf!(
                "{}: {} نتيجة في {} ملي ثانية{tail}",
                "{}: {} results in {} ms{tail}",
                fb.status(&res.text),
                fmt_num(n),
                fmt_num(res.ms)
            )),
            None => self.set_status(&trf!("وجدت {} نتيجة في {} ملي ثانية{tail}", "Found {} results in {} ms{tail}", fmt_num(n), fmt_num(res.ms))),
        }
    }

    pub(super) fn search_content(&self) {
        self.cancel_content();
        let q = self.parse_query(&self.query.text());
        if !q.has_text_criteria() {
            self.set_status(tr!("اكتب الكلمة أو العبارة التي تريد البحث عنها داخل المستندات.", "Type the word or phrase to search for inside documents."));
            return;
        }
        let mut st = self.state.borrow_mut();
        st.shown_query = Some(self.parse_query(&self.query.text()));
        st.history.add_recent(&self.query.text());
        let index = st.index.clone();
        let scope = st.scope.clone();
        let progress = Arc::new(ContentProgress::default());
        let hits = Arc::new(Mutex::new(Vec::new()));
        let (p, h, idx) = (progress.clone(), hits.clone(), index.clone());
        let use_index = st.settings.content_index;
        std::thread::spawn(move || {
            let cache = if use_index { content_index::shared() } else { None };
            let started = std::time::Instant::now();
            search::search_content(&idx.entries, &q, &scope, &p, &h, cache);
            log(&format!(
                "content search: {} documents ({} from the content index) in {} ms",
                p.checked.load(Ordering::Relaxed),
                p.from_cache.load(Ordering::Relaxed),
                started.elapsed().as_millis()
            ));
        });
        st.content = Some(ContentJob { progress, hits });
        st.rows.clear();
        st.rows_index = index;
        drop(st);
        self.clear_selection();
        self.refresh_list();
        self.stop_btn.set_enabled(true);
        let warn = warnings_text(self.state.borrow().shown_query.as_ref());
        self.set_status(&trf!("جاري البحث داخل المستندات…{warn}", "Searching inside documents…{warn}"));
    }

    pub(super) fn cancel_content(&self) {
        if let Some(job) = self.state.borrow_mut().content.take() {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.stop_btn.set_enabled(false);
    }

    pub(super) fn on_stop(&self) {
        let n = self.state.borrow().rows.len();
        self.cancel_content();
        self.set_status(&trf!("تم إيقاف البحث. وجدت {} ملف.", "Search stopped. Found {} files.", fmt_num(n)));
    }

    /// Polls background jobs a few times per second.
    pub(super) fn on_tick(&self) {
        // The first tick handled by the message loop, after the first paint.
        if LOOP_STARTED.load(Ordering::Relaxed) && !self.startup_logged.replace(true) {
            phase("first-tick");
            log_phases();
        }
        self.poll_load();
        self.poll_scan();
        self.poll_content();
        self.apply_watched();
        self.poll_update_progress();
        // Look for a new version about 10 s after start (off the busy
        // startup) and then every hour, for copies that live in the tray.
        let t = self.ticks.get().wrapping_add(1);
        self.ticks.set(t);
        if t % 18_000 == 50 {
            self.maybe_check_for_update();
        }
    }

    pub(super) fn poll_scan(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.scan else { return };
        let finished = job.result.lock().ok().and_then(|mut r| r.take());
        match finished {
            None => {
                let n = job.progress.files.load(Ordering::Relaxed);
                drop(st);
                self.set_status(&trf!("جاري فهرسة الملفات… {} عنصر حتى الآن", "Indexing files… {} items so far", fmt_num(n)));
            }
            Some(idx) => {
                let n = idx.entries.len();
                st.index = Arc::new(idx);
                st.scan = None;
                drop(st);
                self.start_warm();
                self.reindex_btn.set_enabled(true);
                self.start_watching();
                if self.query.text().trim().is_empty() || self.content_mode() {
                    self.set_status(&trf!("اكتملت الفهرسة: {} عنصر.", "Indexing complete: {} items.", fmt_num(n)));
                } else {
                    self.search_names();
                }
            }
        }
    }

    pub(super) fn poll_content(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.content else { return };
        let new_hits: Vec<ContentHit> = job.hits.lock().map(|mut h| h.drain(..).collect()).unwrap_or_default();
        let total = job.progress.total.load(Ordering::Relaxed);
        let checked = job.progress.checked.load(Ordering::Relaxed);
        let done = job.progress.done.load(Ordering::Relaxed);
        let cached = job.progress.from_cache.load(Ordering::Relaxed);
        let changed = !new_hits.is_empty();
        st.rows.extend(new_hits.into_iter().map(|h| Row {
            entry: h.index,
            snippet: Some(h.snippet),
            matches: Some(Box::new(Matches { count: h.count, lines: h.lines })),
        }));
        let n = st.rows.len();
        if done {
            st.content = None;
        }
        drop(st);
        if done {
            self.sort_rows_in_place();
        }
        if changed || done {
            self.refresh_list();
        }
        if done {
            self.stop_btn.set_enabled(false);
            let cached = if cached > 0 { trf!(" (من الفهرس: {})", " ({} from the index)", fmt_num(cached)) } else { String::new() };
            self.set_status(&trf!("انتهى البحث داخل {} مستند: وجدت {} ملف.{}", "Searched {} documents: found {} files.{}", fmt_num(total), fmt_num(n), cached));
        } else {
            self.set_status(&trf!(
                "جاري البحث داخل المستندات… تم فحص {} من {} — وجدت {}",
                "Searching inside documents… checked {} of {} — found {}",
                fmt_num(checked),
                fmt_num(total),
                fmt_num(n)
            ));
        }
    }

    // ---------------------------------------------------------------- scope

    pub(super) fn choose_folder(&self) {
        let mut slot = self.folder_dialog.borrow_mut();
        if slot.is_none() {
            let mut d = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title(tr!("اختر المجلد الذي تريد البحث فيه", "Choose the folder to search in"))
                .action(nwg::FileDialogAction::OpenDirectory)
                .build(&mut d);
            if built.is_err() {
                return;
            }
            *slot = Some(d);
        }
        let Some(dialog) = slot.as_ref() else { return };
        let chosen = if dialog.run(Some(&self.window)) { dialog.get_selected_item().ok() } else { None };
        drop(slot);
        if let Some(dir) = chosen {
            let mut dir = dir.to_string_lossy().into_owned();
            if !dir.ends_with('\\') {
                dir.push('\\');
            }
            self.set_scope(dir);
        }
    }

    pub(super) fn set_scope(&self, scope: String) {
        self.scope_input.set_text(if scope.is_empty() { tr!("كل الأقراص", "All drives") } else { &scope });
        self.state.borrow_mut().scope = scope;
        if !self.query.text().trim().is_empty() && !self.content_mode() {
            self.search_names();
        }
    }
}

/// " — تنبيه: …" for parts of the query that could not be understood, or "".
fn warnings_text(q: Option<&Query>) -> String {
    match q {
        Some(q) if !q.warnings.is_empty() => trf!(" — تنبيه: {}", " — Note: {}", q.warnings.join(tr!("، ", "; "))),
        _ => String::new(),
    }
}
