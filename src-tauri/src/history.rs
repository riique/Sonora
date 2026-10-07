//! Persistent transcription history.
//!
//! The legacy JSON is a read-only baseline. Durable JSONL transactions are
//! replayed into an indexed cache; pages omit heavy attempt details. Deletes
//! remain recoverable. A partial transaction fails closed until explicit repair.

use crate::models::HistoryEntry;
use crate::pipeline_contract::{ContentType, TranscriptionMode};
use crate::pipeline_run::{AudioTransport, PipelineRun, PIPELINE_RUN_SCHEMA_VERSION};
use parking_lot::Mutex;
use std::{fs, path::PathBuf, sync::OnceLock};

static HISTORY_PATH: OnceLock<PathBuf> = OnceLock::new();
static HISTORY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Called once during app setup with the resolved Tauri app data directory.
/// Stores the path so every subsequent helper can read/write without having
/// to thread the `AppHandle` through the audio pipeline.
pub fn init(data_dir: PathBuf) {
    let _ = HISTORY_PATH.set(data_dir);
    let _ = HISTORY_LOCK.set(Mutex::new(()));
}

fn path() -> Option<&'static PathBuf> {
    HISTORY_PATH.get()
}

fn lock() -> &'static Mutex<()> {
    HISTORY_LOCK.get_or_init(|| Mutex::new(()))
}

fn legacy_mode(value: Option<&str>) -> TranscriptionMode {
    match value.unwrap_or_default() {
        "fast-accurate" => TranscriptionMode::FastAccurate,
        "precise" => TranscriptionMode::Precise,
        "ultra-precise" => TranscriptionMode::UltraPrecise,
        _ => TranscriptionMode::UltraFast,
    }
}

fn legacy_transport(entry: &HistoryEntry) -> AudioTransport {
    match entry
        .gemini_transport
        .as_deref()
        .or(entry.deepgram_mode.as_deref())
    {
        Some("multipart") => AudioTransport::Multipart,
        Some("files_api") | Some("resumable_file") => AudioTransport::ResumableFile,
        Some("streaming_final") | Some("websocket_stream") => AudioTransport::WebSocketStream,
        Some("raw_binary") | Some("batch") => AudioTransport::RawBinary,
        Some("url") => AudioTransport::Url,
        _ => AudioTransport::InlineBase64,
    }
}

fn migrate_entry(entry: &mut HistoryEntry) -> bool {
    let mut changed = entry.schema_version < PIPELINE_RUN_SCHEMA_VERSION;
    if entry.pipeline_runs.is_empty() {
        let mode = legacy_mode(entry.mode.as_deref());
        let mut run = if entry.is_error.unwrap_or(false) {
            PipelineRun::hard_error(
                format!("{}-run-legacy", entry.id),
                mode,
                entry
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "legacy pipeline failure".into()),
            )
        } else {
            PipelineRun::success(format!("{}-run-legacy", entry.id), mode, entry.text.clone())
        };
        run.schema_version = PIPELINE_RUN_SCHEMA_VERSION;
        run.session_id = format!("{}-session-legacy", entry.id);
        run.history_engine_label = entry.engine.clone();
        run.model = entry.model.clone().unwrap_or_else(|| entry.engine.clone());
        run.gemini_transport = entry
            .gemini_transport
            .clone()
            .or_else(|| entry.deepgram_mode.clone());
        run.stages = entry
            .stages
            .as_deref()
            .map(|stages| {
                stages
                    .split(',')
                    .map(str::trim)
                    .filter(|stage| !stage.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        run.warnings = entry.warnings.clone().unwrap_or_default();
        run.used_fallback = entry.used_fallback.unwrap_or(false);
        run.fallback_reason = entry.fallback_reason.clone();
        run.whisper_text = entry.whisper_text.clone();
        run.sanitizer_text = entry.sanitizer_text.clone();
        run.gemini_text = entry.gemini_text.clone();
        run.final_text = entry.text.clone();
        run.transcript.raw = entry
            .whisper_text
            .clone()
            .or_else(|| entry.gemini_text.clone())
            .or_else(|| Some(entry.text.clone()));
        run.transcript.refined = entry
            .sanitizer_text
            .clone()
            .or_else(|| entry.gemini_text.clone())
            .or_else(|| Some(entry.text.clone()));
        run.transcript.formatted = Some(entry.text.clone());
        run.transcript.delivered = Some(entry.text.clone());
        run.transcription_latency_ms = entry.transcription_latency_ms.unwrap_or(entry.latency_ms);
        run.audio_prepare_ms = entry.audio_prepare_ms;
        run.base64_ms = entry.base64_ms;
        run.whisper_ms = entry.whisper_ms;
        run.sanitizer_ms = entry.sanitizer_ms.or(entry.sanitizer_latency_ms);
        run.files_upload_ms = entry.files_upload_ms;
        run.files_poll_ms = entry.files_poll_ms;
        run.files_poll_count = entry.files_poll_count;
        run.gemini_generate_ms = entry.gemini_generate_ms;
        run.gemini_delete_ms = entry.gemini_delete_ms;
        run.strict_literals_ms = entry.strict_literals_ms;
        run.total_pipeline_ms = entry.total_pipeline_ms.or(Some(entry.latency_ms));
        run.reported_total_tokens = entry.total_tokens;
        run.content_hint = entry
            .content_type
            .as_deref()
            .and_then(ContentType::parse_str)
            .unwrap_or_default();
        run.debug_info = entry.debug_info.clone();
        run.normalize();
        if let Some(attempt) = run.attempts.last_mut() {
            attempt.transport = legacy_transport(entry);
            attempt
                .result
                .extra
                .insert("migrated_legacy_history".into(), true.into());
        }
        entry.pipeline_runs.push(run);
        changed = true;
    }
    for run in &mut entry.pipeline_runs {
        if run.schema_version < PIPELINE_RUN_SCHEMA_VERSION {
            run.schema_version = PIPELINE_RUN_SCHEMA_VERSION;
            changed = true;
        }
    }
    if entry.schema_version != PIPELINE_RUN_SCHEMA_VERSION {
        entry.schema_version = PIPELINE_RUN_SCHEMA_VERSION;
        changed = true;
    }
    changed
}

fn merge_updated_entry(existing: &HistoryEntry, updated: &HistoryEntry) -> HistoryEntry {
    let mut replacement = updated.clone();
    for prior_run in &existing.pipeline_runs {
        if !replacement
            .pipeline_runs
            .iter()
            .any(|run| run.id == prior_run.id)
        {
            replacement.pipeline_runs.insert(0, prior_run.clone());
        }
    }
    if replacement.evaluation.is_none() {
        replacement.evaluation = existing.evaluation.clone();
    }
    replacement.schema_version = PIPELINE_RUN_SCHEMA_VERSION;
    replacement
}

/// A complete line is the commit boundary; partial writes are never accepted.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Event {
    Import {
        entries: Vec<HistoryEntry>,
        #[serde(default)]
        deleted: Vec<String>,
    },
    Upsert {
        entry: Box<HistoryEntry>,
    },
    Delete {
        id: String,
    },
    AudioPath {
        id: String,
        path: String,
    },
    Clear,
}
#[derive(Default)]
struct Store {
    entries: std::collections::HashMap<String, HistoryEntry>,
    order: std::collections::VecDeque<String>,
    active: std::collections::HashSet<String>,
}
impl Store {
    fn apply(&mut self, event: Event) {
        match event {
            Event::Import { entries, deleted } => {
                for entry in entries.into_iter().rev() {
                    if !self.entries.contains_key(&entry.id) {
                        self.apply(Event::Upsert {
                            entry: Box::new(entry.clone()),
                        });
                        if deleted.contains(&entry.id) {
                            self.apply(Event::Delete { id: entry.id });
                        }
                    }
                }
            }
            Event::Upsert { entry } => {
                if self.active.insert(entry.id.clone()) {
                    self.order.push_front(entry.id.clone());
                }
                self.entries.insert(entry.id.clone(), *entry);
            }
            Event::Delete { id } => {
                self.active.remove(&id);
                self.order.retain(|old| old != &id);
            }
            Event::AudioPath { id, path } => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.audio_path = Some(path);
                }
            }
            Event::Clear => {
                self.active.clear();
                self.order.clear();
            }
        }
    }
    fn open(file: &std::path::Path) -> Result<Self, String> {
        use std::io::BufRead;
        let mut store = Self::default();
        let entries: Vec<HistoryEntry> = crate::storage::read_json(file)?;
        for mut entry in entries.into_iter().rev() {
            migrate_entry(&mut entry);
            store.apply(Event::Upsert {
                entry: Box::new(entry),
            });
        }
        let journal = journal_path(file);
        if journal.exists() {
            let mut reader =
                std::io::BufReader::new(fs::File::open(journal).map_err(|e| e.to_string())?);
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                    break;
                }
                if !line.ends_with('\n') {
                    return Err("Histórico interrompido durante a gravação. Use Reparar histórico em Recuperação; o original será preservado.".into());
                }
                store.apply(serde_json::from_str(&line).map_err(|_| {
                    "Registro de histórico inválido; restaure um backup antes de salvar"
                })?);
            }
        }
        Ok(store)
    }
}
type Stamp = (
    Option<(u64, std::time::SystemTime)>,
    Option<(u64, std::time::SystemTime)>,
);
struct Cached {
    stamp: Stamp,
    store: Store,
}
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
fn journal_path(file: &std::path::Path) -> PathBuf {
    file.with_extension("events.jsonl")
}
fn stamp(file: &std::path::Path) -> Stamp {
    let metadata = |path: &std::path::Path| {
        fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)))
    };
    (metadata(file), metadata(&journal_path(file)))
}
fn access<T>(action: impl FnOnce(&Store) -> T) -> Result<T, String> {
    let _guard = lock().lock();
    let file = path().ok_or("Diretório de histórico indisponível")?;
    let mut cache = CACHE.lock();
    let current = stamp(file);
    if cache.as_ref().is_none_or(|cached| cached.stamp != current) {
        *cache = Some(Cached {
            stamp: current,
            store: Store::open(file)?,
        });
    }
    Ok(action(
        &cache.as_ref().ok_or("Histórico indisponível")?.store,
    ))
}
fn mutate(build: impl FnOnce(&Store) -> Result<Event, String>) -> Result<(), String> {
    use std::io::Write;
    let _guard = lock().lock();
    let file = path().ok_or("Diretório de histórico indisponível")?;
    let mut cache = CACHE.lock();
    let current = stamp(file);
    if cache.as_ref().is_none_or(|cached| cached.stamp != current) {
        *cache = Some(Cached {
            stamp: current,
            store: Store::open(file)?,
        });
    }
    let cached = cache.as_mut().ok_or("Histórico indisponível")?;
    let event = build(&cached.store)?;
    let mut bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut journal = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path(file))
        .map_err(|e| e.to_string())?;
    if let Err(error) = journal.write_all(&bytes).and_then(|_| journal.sync_all()) {
        *cache = None;
        return Err(error.to_string());
    }
    cached.store.apply(event);
    cached.stamp = stamp(file);
    Ok(())
}
pub fn try_load_all() -> Result<Vec<HistoryEntry>, String> {
    access(|store| {
        store
            .order
            .iter()
            .filter_map(|id| store.entries.get(id).cloned())
            .collect()
    })
}
pub fn load_all() -> Vec<HistoryEntry> {
    try_load_all().unwrap_or_else(|error| {
        log::error!("history: {error}");
        Vec::new()
    })
}
pub fn get(id: &str) -> Option<HistoryEntry> {
    access(|store| {
        store
            .active
            .contains(id)
            .then(|| store.entries.get(id).cloned())
            .flatten()
    })
    .ok()
    .flatten()
}
pub fn push(mut entry: HistoryEntry) -> Result<(), String> {
    if let Some(error) = &mut entry.error_message {
        *error = crate::redaction::message(error);
    }
    for run in &mut entry.pipeline_runs {
        for attempt in &mut run.attempts {
            if let Some(error) = &mut attempt.error {
                error.message = crate::redaction::message(&error.message);
            }
        }
    }
    mutate(|_| {
        Ok(Event::Upsert {
            entry: Box::new(entry.clone()),
        })
    })?;
    crate::insights::enqueue_entry(entry);
    Ok(())
}
pub fn update_entry(updated: HistoryEntry) -> bool {
    let result = mutate(|store| {
        let existing = store
            .entries
            .get(&updated.id)
            .filter(|_| store.active.contains(&updated.id))
            .ok_or("Entrada não encontrada")?;
        Ok(Event::Upsert {
            entry: Box::new(merge_updated_entry(existing, &updated)),
        })
    });
    if result.is_ok() {
        crate::insights::enqueue_entry(updated);
    }
    result.is_ok()
}
fn edit_entry(id: &str, edit: impl FnOnce(&mut HistoryEntry)) -> bool {
    let mut changed = None;
    let result = mutate(|store| {
        let mut entry = store
            .entries
            .get(id)
            .filter(|_| store.active.contains(id))
            .ok_or("Entrada não encontrada")?
            .clone();
        edit(&mut entry);
        changed = Some(entry.clone());
        Ok(Event::Upsert {
            entry: Box::new(entry),
        })
    });
    if let (Ok(()), Some(entry)) = (&result, changed) {
        crate::insights::enqueue_entry(entry);
    }
    result.is_ok()
}
pub fn update_text(id: &str, text: &str) -> bool {
    edit_entry(id, |entry| {
        entry.text = text.into();
        entry.words = text.split_whitespace().count();
        entry.is_error = Some(false);
        entry.error_message = None;
        if let Some(run) = entry.pipeline_runs.last_mut() {
            run.transcript.set_user_corrected(text);
        }
    })
}
pub fn set_evaluation(id: &str, feedback: &str) -> bool {
    edit_entry(id, |entry| entry.evaluation = Some(feedback.into()))
}
pub fn clear() -> Result<(), String> {
    mutate(|_| Ok(Event::Clear))?;
    crate::insights::enqueue_clear();
    Ok(())
}
pub fn delete_entry(id: &str) -> bool {
    let result = mutate(|store| {
        if !store.active.contains(id) {
            return Err("Entrada não encontrada".into());
        }
        Ok(Event::Delete { id: id.into() })
    });
    if result.is_ok() {
        crate::insights::enqueue_remove(id);
    }
    result.is_ok()
}
pub fn restore_entry(id: &str) -> Result<(), String> {
    mutate(|store| {
        let entry = store
            .entries
            .get(id)
            .ok_or("Entrada removida não encontrada")?;
        if store.active.contains(id) {
            return Err("Entrada já está no histórico".into());
        }
        Ok(Event::Upsert {
            entry: Box::new(entry.clone()),
        })
    })?;
    if let Some(entry) = get(id) {
        crate::insights::enqueue_entry(entry);
    }
    Ok(())
}
#[derive(serde::Serialize)]
pub struct Page {
    pub items: Vec<HistoryEntry>,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub total_words: usize,
    /// Reported/estimated provider cost of this calendar month's dictations.
    pub month_cost_usd: Option<f64>,
    pub month_dictations: usize,
}

/// Sum of the costs of every pipeline run (retries are billed too).
pub fn entry_cost_usd(entry: &HistoryEntry) -> Option<f64> {
    let costs = entry
        .pipeline_runs
        .iter()
        .filter_map(|run| run.usage.cost.amount_usd)
        .collect::<Vec<_>>();
    (!costs.is_empty()).then(|| costs.iter().sum())
}

fn month_totals<'a>(
    entries: impl Iterator<Item = &'a HistoryEntry>,
    month: &str,
) -> (Option<f64>, usize) {
    let mut cost = None;
    let mut count = 0;
    for entry in entries.filter(|entry| entry.date.starts_with(month)) {
        count += 1;
        if let Some(amount) = entry_cost_usd(entry) {
            cost = Some(cost.unwrap_or(0.0) + amount);
        }
    }
    (cost, count)
}
fn project(mut entry: HistoryEntry) -> HistoryEntry {
    entry.debug_info = None;
    // Keep delivery outcome and latest transcript versions available to row actions.
    if let Some(mut run) = entry.pipeline_runs.pop() {
        run.debug_info = None;
        run.attempts.clear();
        run.journal.clear();
        entry.pipeline_runs = vec![run];
    }
    entry
}
pub fn page(query: &str, offset: usize, limit: usize, deleted: bool) -> Result<Page, String> {
    access(|store| project_page(store, query, offset, limit, deleted))
}
fn project_page(store: &Store, query: &str, offset: usize, limit: usize, deleted: bool) -> Page {
    let query = query.trim().to_lowercase();

    let matches = |entry: &&HistoryEntry| {
        query.is_empty()
            || entry.text.to_lowercase().contains(&query)
            || entry
                .error_message
                .as_ref()
                .is_some_and(|error| error.to_lowercase().contains(&query))
    };
    let entries: Vec<&HistoryEntry> = if deleted {
        let mut entries: Vec<_> = store
            .entries
            .values()
            .filter(|entry| !store.active.contains(&entry.id))
            .filter(matches)
            .collect();
        entries.sort_by(|a, b| b.date.cmp(&a.date));
        entries
    } else {
        store
            .order
            .iter()
            .filter_map(|id| store.entries.get(id))
            .filter(matches)
            .collect()
    };
    let total = entries.len();
    let limit = limit.clamp(1, 100);
    let items = entries
        .iter()
        .skip(offset)
        .take(limit)
        .map(|entry| project((*entry).clone()))
        .collect();
    let today = crate::audio::now_timestamp();
    let month = today.get(..7).unwrap_or_default();
    let (month_cost_usd, month_dictations) = if deleted {
        (None, 0)
    } else {
        month_totals(
            store.order.iter().filter_map(|id| store.entries.get(id)),
            month,
        )
    };
    Page {
        items,
        total,
        month_cost_usd,
        month_dictations,
        total_words: entries.iter().map(|entry| entry.words).sum(),
        next_offset: (offset.saturating_add(limit) < total).then_some(offset.saturating_add(limit)),
    }
}
/// Repairs only an incomplete trailing transaction, preserving a byte-exact backup.
pub fn repair_journal() -> Result<(), String> {
    let _guard = lock().lock();
    let file = journal_path(path().ok_or("Histórico indisponível")?);
    repair_at(&file)?;
    *CACHE.lock() = None;
    Ok(())
}
fn repair_at(file: &std::path::Path) -> Result<(), String> {
    let bytes = fs::read(file).map_err(|e| e.to_string())?;
    if bytes.ends_with(b"\n") || bytes.is_empty() {
        return Err("Nenhuma transação incompleta encontrada".into());
    }
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    for line in bytes[..end]
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
    {
        serde_json::from_slice::<Event>(line)
            .map_err(|_| "Há corrupção anterior ao último registro; restaure um backup")?;
    }
    crate::storage::atomic_write(file, &bytes[..end])?;
    Ok(())
}

pub fn import_entries(
    mut entries: Vec<HistoryEntry>,
    deleted: Vec<String>,
) -> Result<usize, String> {
    let mut count = 0;
    mutate(|store| {
        entries.retain(|entry| !store.entries.contains_key(&entry.id));
        for entry in &mut entries {
            migrate_entry(entry);
        }
        count = entries.len();
        Ok(Event::Import { entries, deleted })
    })?;
    Ok(count)
}

pub fn get_including_deleted(id: &str) -> Result<HistoryEntry, String> {
    access(|store| store.entries.get(id).cloned())?.ok_or("Entrada não encontrada".into())
}
pub fn archive_audio_path(id: &str, path: String) -> Result<(), String> {
    mutate(|store| {
        if !store.entries.contains_key(id) {
            return Err("Entrada não encontrada".into());
        }
        Ok(Event::AudioPath {
            id: id.into(),
            path,
        })
    })
}

pub fn export_entries() -> Result<(Vec<HistoryEntry>, Vec<String>), String> {
    access(|store| {
        let deleted: Vec<_> = store
            .entries
            .keys()
            .filter(|id| !store.active.contains(*id))
            .cloned()
            .collect();
        let mut entries: Vec<_> = store
            .order
            .iter()
            .filter_map(|id| store.entries.get(id).cloned())
            .collect();
        entries.extend(
            deleted
                .iter()
                .filter_map(|id| store.entries.get(id).cloned()),
        );
        (entries, deleted)
    })
}

#[cfg(test)]
mod cost_tests {
    use super::*;

    #[test]
    fn month_totals_sum_runs_of_matching_month_only() {
        let entry_at = |date: &str| -> HistoryEntry {
            serde_json::from_value(serde_json::json!({
                "id": date,
                "date": date,
                "words": 1,
                "engine": "GroqWhisper",
                "text": "oi"
            }))
            .unwrap()
        };
        let mut entry = entry_at("2026-10-07 10:00");
        let mut run = PipelineRun::default();
        run.usage.cost.amount_usd = Some(0.002);
        entry.pipeline_runs = vec![run.clone(), run];
        let old = entry_at("2026-09-30 10:00");
        let (cost, count) = month_totals([&entry, &old].into_iter(), "2026-10");
        assert_eq!(count, 1);
        assert!((cost.unwrap() - 0.004).abs() < 1e-12);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_entry() -> HistoryEntry {
        serde_json::from_value(serde_json::json!({
            "id": "legacy-1",
            "date": "2026-08-25 10:00",
            "words": 2,
            "engine": "GroqWhisper",
            "text": "Open Router",
            "whisper_text": "open router",
            "sanitizer_text": "Open Router",
            "model": "whisper-large-v3-turbo",
            "stages": "whisper,sanitizer",
            "total_tokens": 4
        }))
        .unwrap()
    }

    #[test]
    fn legacy_entry_migrates_without_losing_projection() {
        let mut entry = legacy_entry();
        assert!(migrate_entry(&mut entry));
        assert_eq!(entry.schema_version, PIPELINE_RUN_SCHEMA_VERSION);
        assert_eq!(entry.text, "Open Router");
        assert_eq!(entry.pipeline_runs.len(), 1);
        let run = &entry.pipeline_runs[0];
        assert_eq!(run.transcript.raw.as_deref(), Some("open router"));
        assert_eq!(run.transcript.refined.as_deref(), Some("Open Router"));
        assert_eq!(run.attempts.len(), 1);
        assert_eq!(run.journal.len(), 2);
    }

    #[test]
    fn retry_merge_preserves_previous_pipeline_run() {
        let mut existing = legacy_entry();
        migrate_entry(&mut existing);
        let original_run_id = existing.pipeline_runs[0].id.clone();

        let mut updated = existing.clone();
        updated.pipeline_runs.clear();
        updated.pipeline_runs.push(PipelineRun::success(
            "legacy-1-run-retry",
            TranscriptionMode::UltraFast,
            "OpenRouter",
        ));
        let merged = merge_updated_entry(&existing, &updated);
        assert_eq!(merged.pipeline_runs.len(), 2);
        assert_eq!(merged.pipeline_runs[0].id, original_run_id);
        assert_eq!(merged.pipeline_runs[1].id, "legacy-1-run-retry");
    }
    #[test]
    fn journal_replays_delete_restore_and_preserves_incomplete_original() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("sonora-history-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history.json");
        crate::storage::write_json(&file, &vec![legacy_entry()]).unwrap();
        let journal = journal_path(&file);
        let mut writer = fs::File::create(&journal).unwrap();
        let deletion = serde_json::to_vec(&Event::Delete {
            id: "legacy-1".into(),
        })
        .unwrap();
        writer.write_all(&deletion).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.sync_all().unwrap();
        let store = Store::open(&file).unwrap();
        assert!(store.active.is_empty());
        assert_eq!(project_page(&store, "router", 0, 50, true).total, 1);
        writer.write_all(b"{\"kind\":").unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        assert!(Store::open(&file).is_err());
        let original = fs::read(&journal).unwrap();
        repair_at(&journal).unwrap();
        assert_eq!(fs::read(journal.with_extension("bak")).unwrap(), original);
        let mut store = Store::open(&file).unwrap();
        store.apply(Event::Upsert {
            entry: Box::new(legacy_entry()),
        });
        assert_eq!(
            project_page(&store, "", 0, 1, false).items[0].id,
            "legacy-1"
        );
        fs::write(&journal, b"bad\n{partial").unwrap();
        assert!(repair_at(&journal).is_err());
        assert_eq!(
            fs::read(&file).unwrap(),
            serde_json::to_vec_pretty(&vec![legacy_entry()]).unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn history_pages_at_ten_and_hundred_thousand_entries() {
        let mut store = Store::default();
        for count in [10_000, 100_000] {
            let start = std::time::Instant::now();
            for index in store.entries.len()..count {
                let mut entry = legacy_entry();
                entry.id = format!("synthetic-{index}");
                store.apply(Event::Upsert {
                    entry: Box::new(entry),
                });
            }
            let populate_ms = start.elapsed().as_millis();
            let page_start = std::time::Instant::now();
            let page = project_page(&store, "router", 49_950.min(count - 50), 50, false);
            assert_eq!(page.items.len(), 50);
            assert_eq!(page.total, count);
            let bytes = serde_json::to_vec(&page).unwrap().len();
            assert!(bytes < 256_000);
            assert!(
                page_start.elapsed().as_secs() < 5,
                "pagination exceeded 5s budget"
            );
            eprintln!(
                "history corpus={count} populate_ms={populate_ms} page_ms={} payload_bytes={bytes}",
                page_start.elapsed().as_millis()
            );
        }
    }
}
