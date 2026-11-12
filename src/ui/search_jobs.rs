//! Loading and scanning the index, and the filename and document searches,
//! all running on background threads that the timer polls.

use native_windows_gui as nwg;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::content_index;
use super::files::{EDIT_SAVE, SAVES};
use super::format::fmt_num;
use super::startup::{LOOP_STARTED, background_priority, log, log_phases, phase};
use super::{App, ContentJob, LoadJob, Matches, Row, ScanJob};
use crate::index::{self, Index, ScanProgress};
use crate::fuzzy;
use crate::indexing_status;
use crate::search::{self, ContentHit, ContentProgress, Query, SortKey};
use crate::system_files;

/// The outcome of a background filename search.
pub(super) struct NameResult {
    index: Arc<Index>,
    hits: Vec<u32>,
    text: String,
    /// How a fallback (keyboard layout, Arabizi, typos) found the hits, if one did.
    converted: Option<fuzzy::Fallback>,
    /// A text that finds something, when this one found nothing at all.
    did_you_mean: Option<String>,
    empty: bool,
    ms: usize,
}

impl App {
    // ---------------------------------------------------------------- index

    /// Reads the cached index on a background thread so the window appears at once.
    pub(super) fn start(&self) {
        // The last session ended with changes it could not save (it was
        // killed, or gave up on a slow disk): the saved index is whole but
        // misses them, so the drives are scanned again once it is loaded.
        if crate::index_save::marker_path(&index::cache_path()).exists() {
            SAVES.scan_owed();
            self.reconcile.set(true);
        }
        let slot: LoadJob = Arc::new(Mutex::new(None));
        let s = slot.clone();
        let started_thread = crate::crash_log::spawn_named("index-load", move || {
            background_priority();
            let started = std::time::Instant::now();
            // A panic while reading a damaged cache counts as no cache, so
            // the window builds a new index instead of loading forever.
            let idx = crate::crash_log::catch(|| Index::load(&index::cache_path())).flatten();
            log(&format!(
                "index load: {} entries in {} ms",
                idx.as_ref().map_or(0, |i| i.entries.len()),
                started.elapsed().as_millis()
            ));
            if let Ok(mut slot) = s.lock() {
                *slot = Some(idx);
            }
        });
        if started_thread.is_none() {
            // No thread to read the cache: go straight to a fresh scan.
            if let Ok(mut s) = slot.lock() {
                *s = Some(None);
            }
        }
        self.state.borrow_mut().loading = Some(slot);
        self.set_status(tr!("جارٍ تحميل الفهرس…", "Loading the index…"));
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
                    self.show_recent_files();
                } else {
                    self.search_names();
                }
                if self.reconcile.take() {
                    log("start: the last session ended with index changes it had not saved; scanning the drives again");
                    self.start_scan();
                }
            }
            None => {
                drop(st);
                self.start_scan();
            }
        }
    }

    pub(super) fn start_scan(&self) {
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
        // An edit still waiting to be saved must not land after the new
        // scan: it is dropped here (the scan sees its change on disk and
        // writes a newer index, and saves are numbered, so an older one never
        // replaces a newer file). If the scan fails it is queued again. The
        // UI thread no longer writes it here itself.
        SAVES.discard();
        let progress = Arc::new(ScanProgress::default());
        let result = Arc::new(Mutex::new(None));
        let (p, r) = (progress.clone(), result.clone());
        let started_thread = crate::crash_log::spawn_named("index-build", move || {
            background_priority();
            // A save still being written holds a snapshot of the old index:
            // let it end first, so the walk's new index is the second copy
            // in memory and not the third. (Not for ever: a stuck disk must
            // not keep the scan from starting.)
            if !SAVES.wait_idle(std::time::Duration::from_secs(60)) {
                log("index build: a save was still running after 60 s; scanning anyway");
            }
            let started = std::time::Instant::now();
            // `None` after a panic (logged by the hook): the scan still ends,
            // keeping the index there was, instead of indexing forever.
            let idx = crate::crash_log::catch(|| Index::build(&index::list_drives(), &p));
            if let Some(idx) = &idx {
                log(&format!("index build: {} entries in {} ms", idx.entries.len(), started.elapsed().as_millis()));
            }
            // A scan stopped half-way (the program is closing) has only part
            // of the files: it must not replace the saved index.
            if let Some(idx) = idx.as_ref().filter(|_| !p.cancel.load(Ordering::Relaxed)) {
                let saving = std::time::Instant::now();
                match SAVES.save_now(idx) {
                    Ok(()) => log(&format!("index save: {} entries in {} ms (after the scan)", idx.entries.len(), saving.elapsed().as_millis())),
                    Err(e) => {
                        // Queued again once the window has the new index
                        // (`poll_scan`): retried, and marked as unsaved.
                        SCAN_SAVE_FAILED.store(true, Ordering::Relaxed);
                        log(&format!("index save failed after the scan: {e}; it will be tried again"));
                    }
                }
            }
            if let Ok(mut slot) = r.lock() {
                *slot = Some(idx);
            }
        });
        if started_thread.is_none() {
            SAVES.set_pending(st.index.clone(), EDIT_SAVE);
            drop(st);
            self.start_watching();
            self.set_status(tr!("تعذرت فهرسة الملفات.", "Could not index the files."));
            return;
        }
        st.scan = Some(ScanJob { progress, result });
        drop(st);
        self.reindex_btn.set_enabled(false);
        self.set_status(tr!("جارٍ فهرسة الملفات على كل الأقراص…", "Indexing files on all drives…"));
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
        self.show_app_matches();
    }

    pub(super) fn on_search(&self) {
        if self.content_mode() {
            self.search_content();
        } else {
            self.search_names();
        }
    }

    /// Switches "hide system files" and runs the current search again.
    pub(super) fn toggle_hide_system(&self) {
        let hide = !self.menu_hide_system.checked();
        self.menu_hide_system.set_checked(hide);
        self.state.borrow_mut().settings.hide_system = hide;
        if let Some(shared) = self.web_running_shared() {
            self.set_web_show_system(&shared);
        }
        if !self.content_mode() || !self.query.text().trim().is_empty() {
            self.on_search();
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
        let show_system = system_files::show_system(st.settings.hide_system, &scope);
        // A spelling suggestion is shown only for a search the user ran
        // while the index is ready (see `on_names_found`): don't look for one
        // otherwise.
        let suggest = !st.live_search && st.scan.is_none() && st.loading.is_none();
        drop(st);

        let text = self.query.text();
        let kind = self.kind();
        let parse = move |t: &str| {
            let mut q = Query::parse(t);
            q.apply_kind(kind);
            q.show_system |= show_system;
            q
        };
        let slot = self.name_result.clone();
        let notice = self.name_notice.sender();
        let started_thread = crate::crash_log::spawn_named("name-search", move || {
            let started = std::time::Instant::now();
            let run = || {
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
                // Nothing at all: a text that finds something, to suggest.
                let did_you_mean = if suggest && hits.is_empty() && !q.is_empty() { crate::suggest::did_you_mean(&index.entries, &text, &scope, &parse) } else { None };
                (hits, converted, did_you_mean, q.is_empty())
            };
            // After a panic (logged by the hook) the search still answers,
            // with no results: otherwise `name_running` would stay set and
            // no search would run again, nor any change from the watcher.
            let (hits, converted, did_you_mean, empty) = crate::crash_log::catch(run).unwrap_or_default();
            let ms = started.elapsed().as_millis() as usize;
            if let Ok(mut r) = slot.lock() {
                *r = Some(NameResult { index, hits, text, converted, did_you_mean, empty, ms });
            }
            notice.notice();
        });
        if started_thread.is_none() {
            let mut st = self.state.borrow_mut();
            st.name_running = false;
            st.name_pending = false;
        }
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
        st.rows = if res.empty {
            // The empty box lists the files opened lately.
            super::empty_state::recent_rows(&st.history, &res.index)
        } else {
            res.hits.into_iter().map(|entry| Row { entry, snippet: None, matches: None }).collect()
        };
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
            if n > 0 || res.empty {
                self.show_suggestions(Vec::new());
            }
            if n != old_n && !res.empty {
                self.set_status(&trf!("وجدت {} نتيجة (حُدِّثت تلقائياً){warn}", "Found {} results (updated automatically){warn}", fmt_num(n)));
            }
            return;
        }
        self.clear_selection();
        self.refresh_list();
        if n == 0 && !res.empty && !busy {
            self.suggest_for_names(&res.text, res.did_you_mean.clone());
        } else {
            self.show_suggestions(Vec::new());
        }
        if res.empty {
            if !busy {
                self.set_status(if n > 0 {
                    tr!("اكتب اسم الملف أو جزءاً منه للبحث. في القائمة الملفات التي فتحتها مؤخراً.", "Type a file name or part of one to search. Listed: the files you opened lately.")
                } else {
                    tr!("اكتب اسم الملف أو جزءاً منه للبحث.", "Type a file name or part of one to search.")
                });
            }
            return;
        }
        // Nothing yet while the drives are still being scanned: not "no
        // results", the index is not complete.
        if n == 0
            && busy
            && !loading
            && let Some(text) = self.no_results_while_indexing()
        {
            self.set_status(&format!("{text}{warn}"));
            return;
        }
        let tail = match (loading, busy) {
            (true, _) => tr!(" (جارٍ تحميل الفهرس…)", " (loading the index…)"),
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
        let mut q = self.parse_query(&self.query.text());
        if !q.has_text_criteria() {
            self.set_status(tr!("اكتب الكلمة أو العبارة التي تريد البحث عنها داخل المستندات.", "Type the word or phrase to search for inside documents."));
            return;
        }
        let mut st = self.state.borrow_mut();
        let mut shown = self.parse_query(&self.query.text());
        shown.note_document_search();
        st.shown_query = Some(shown);
        st.history.add_recent(&self.query.text());
        let index = st.index.clone();
        let scope = st.scope.clone();
        q.show_system |= system_files::show_system(st.settings.hide_system, &scope);
        let progress = Arc::new(ContentProgress::default());
        let hits = Arc::new(Mutex::new(Vec::new()));
        let (p, h, idx) = (progress.clone(), hits.clone(), index.clone());
        let use_index = st.settings.content_index;
        crate::crash_log::spawn_named("content-search", move || {
            let cache = if use_index { content_index::shared() } else { None };
            let started = std::time::Instant::now();
            // After a panic (logged by the hook) the search still ends, so
            // the window doesn't wait for it forever.
            if crate::crash_log::catch(|| search::search_content(&idx.entries, &q, &scope, &p, &h, cache)).is_none() {
                p.done.store(true, Ordering::Relaxed);
            }
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
        self.show_suggestions(Vec::new());
        self.clear_selection();
        self.refresh_list();
        self.stop_btn.set_enabled(true);
        let warn = warnings_text(self.state.borrow().shown_query.as_ref());
        self.set_status(&trf!("جارٍ البحث داخل المستندات…{warn}", "Searching inside documents…{warn}"));
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
            // This copy runs: the old program file of an update may go now.
            crate::crash_log::spawn_named("update-leftovers", || {
                if let Some(line) = crate::update::remove_update_leftovers() {
                    log(&line);
                }
            });
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
        self.poll_indexing(t);
        self.memory_tick();
        self.webui_tick(t);
        if t % 18_000 == 50 {
            self.maybe_check_for_update();
        }
        // Offer to report a crash of the last run once the window is up
        // (not while the program waits hidden in the tray).
        if t >= 10 && !self.crash_checked.get() && self.main_shown() {
            self.crash_checked.set(true);
            self.offer_crash_report();
        }
        // Then, once, the offer to move a copy out of Downloads. It comes
        // before the first-run question about the floating box (tick 14 in
        // `quick_tick`, and not while this offer's message box is open), so
        // a copy that moves asks that question from its new place.
        if t >= 12 && self.crash_checked.get() && !self.move_checked.get() && self.main_shown() && self.window.enabled() {
            self.move_checked.set(true);
            self.offer_move();
        }
        self.quick_tick(t);
        self.rating_tick(t);
    }

    pub(super) fn poll_scan(&self) {
        let mut st = self.state.borrow_mut();
        let Some(job) = &st.scan else { return };
        let finished = job.result.lock().ok().and_then(|mut r| r.take());
        match finished {
            // The count shows in the status bar's progress part
            // (`poll_indexing`), so a search's own message stays readable.
            None => {}
            Some(None) => {
                st.scan = None;
                // The index there was stays; edits to it were not queued
                // while the scan ran.
                SAVES.set_pending(st.index.clone(), EDIT_SAVE);
                drop(st);
                self.reindex_btn.set_enabled(true);
                self.start_watching();
                self.set_status(tr!("تعذرت فهرسة الملفات.", "Could not index the files."));
            }
            Some(Some(idx)) => {
                let n = idx.entries.len();
                st.index = Arc::new(idx);
                st.scan = None;
                // The scan could not write its index: the save queue takes it
                // (tries again, and marks it unsaved until it is on disk).
                if SCAN_SAVE_FAILED.swap(false, Ordering::Relaxed) {
                    SAVES.set_pending(st.index.clone(), EDIT_SAVE);
                }
                drop(st);
                self.start_warm();
                self.reindex_btn.set_enabled(true);
                self.start_watching();
                if self.query.text().trim().is_empty() || self.content_mode() {
                    self.set_status(&trf!("اكتملت الفهرسة: {} عنصر.", "Indexing complete: {} items.", fmt_num(n)));
                    self.show_recent_files();
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
        let capped = job.progress.capped.load(Ordering::Relaxed);
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
        // The end of the search puts the rows in order: the selection goes
        // with its file, so Enter or Delete never acts on the file that took
        // its row number.
        let moved = done.then(|| self.sort_rows_in_place(self.list.selected_item()));
        if changed || done {
            self.refresh_list();
        } else if n == 0 {
            // The empty list says how far the reading got.
            self.list.invalidate();
        }
        if let Some(row) = moved {
            self.select_row(row);
        }
        if done {
            self.stop_btn.set_enabled(false);
            let cached = if cached > 0 { trf!(" (من الفهرس: {})", " ({} from the index)", fmt_num(cached)) } else { String::new() };
            let mut status = trf!("انتهى البحث داخل {} مستند: وجدت {} ملف.{}", "Searched {} documents: found {} files.{}", fmt_num(total), fmt_num(n), cached);
            // More documents match than one search lists.
            if capped {
                status = trf!("{status} (تُعرض أول {} نتيجة فقط؛ أضف كلمة أو اختر مجلداً لتضييق البحث.)", "{status} (showing the first {} results only; add a word or pick a folder to narrow the search.)", fmt_num(n));
            }
            // The drives are still being scanned: documents not in the index
            // yet were not searched. (A finished search did read every
            // document the content index lacked, so its warm-up doesn't count.)
            if n == 0 && self.state.borrow().scan.is_some() {
                status = format!("{status} {}.", indexing_status::incomplete_text(crate::i18n::current()));
            }
            // Images were searched without their Arabic text: say so rather
            // than skipping it silently.
            if crate::ocr::arabic_missing() {
                status = format!("{status} {}", indexing_status::ocr_arabic_missing_text(crate::i18n::current()));
            }
            self.set_status(&status);
            if n == 0 {
                self.suggest_for_content(&self.query.text());
            }
        } else if n == 0
            && let Some(text) = self.content_search_while_indexing(checked, total)
        {
            // The first day: a search over every drive reads most documents
            // itself while the content index fills, and finds nothing for a
            // while. Say why, rather than a bare "found 0".
            self.set_status(&text);
        } else {
            self.set_status(&trf!(
                "جارٍ البحث داخل المستندات… تم فحص {} من {} — وجدت {}",
                "Searching inside documents… checked {} of {} — found {}",
                fmt_num(checked),
                fmt_num(total),
                fmt_num(n)
            ));
        }
    }

    // ---------------------------------------------------------------- scope

    pub(super) fn choose_folder(&self) {
        // Taken out while it runs: the dialog pumps messages, and a handler
        // that runs meanwhile must not find the cell borrowed.
        let mut slot = self.folder_dialog.take();
        if slot.is_none() {
            let mut d = nwg::FileDialog::default();
            let built = nwg::FileDialog::builder()
                .title(tr!("اختر المجلد الذي تريد البحث فيه", "Choose the folder to search in"))
                .action(nwg::FileDialogAction::OpenDirectory)
                .build(&mut d);
            if built.is_err() {
                return;
            }
            slot = Some(d);
        }
        let Some(dialog) = slot.as_ref() else { return };
        let chosen = if dialog.run(Some(&self.window)) { dialog.get_selected_item().ok() } else { None };
        *self.folder_dialog.borrow_mut() = slot;
        if let Some(dir) = chosen {
            let mut dir = dir.to_string_lossy().into_owned();
            if !dir.ends_with('\\') {
                dir.push('\\');
            }
            self.set_scope(dir);
        }
    }

    pub(super) fn set_scope(&self, scope: String) {
        self.show_scope(&scope);
        self.state.borrow_mut().scope = scope;
        if !self.query.text().trim().is_empty() && !self.content_mode() {
            self.search_names();
        }
    }
}

/// The last full scan could not save its index (see `poll_scan`).
static SCAN_SAVE_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// " — تنبيه: …" for parts of the query that could not be understood, or "".
fn warnings_text(q: Option<&Query>) -> String {
    match q {
        Some(q) if !q.warnings.is_empty() => trf!(" — تنبيه: {}", " — Note: {}", q.warnings.join(tr!("، ", "; "))),
        _ => String::new(),
    }
}
