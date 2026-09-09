use super::*;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum LearningCommand {
    List,
    Delete {
        reading: String,
        surface: String,
    },
    Save {
        original_reading: Option<String>,
        original_surface: Option<String>,
        reading: String,
        surface: String,
    },
}

fn remove_pair(history: &mut HashMap<String, Vec<LearnEntry>>, reading: &str, surface: &str) {
    if let Some(entries) = history.get_mut(reading) {
        entries.retain(|e| e.surface != surface);
        if entries.is_empty() {
            history.remove(reading);
        }
    }
}

impl DictStore {
    /// Apply settings edits to current host-owned history, never a stale UI snapshot.
    /// Publish only after successful persistence, so errors leave memory unchanged.
    pub fn manage_learning(&self, command: LearningCommand) -> Result<serde_json::Value> {
        let mut history = self
            .inner
            .learn_history
            .write()
            .map_err(|_| anyhow::anyhow!("learning lock failed"))?;
        if matches!(command, LearningCommand::List) {
            let now = now_unix_secs();
            let mut entries: Vec<_> = history
                .iter()
                .flat_map(|(reading, entries)| {
                    entries.iter().map(move |e| {
                        serde_json::json!({"reading": reading, "surface": e.surface,
                    "last_access_time": e.last_access_time,
                    "frequency": e.suggestion_freq as f64 * decay_factor(e.last_access_time, now)})
                    })
                })
                .collect();
            entries.sort_by(|a, b| {
                b["last_access_time"]
                    .as_u64()
                    .cmp(&a["last_access_time"].as_u64())
                    .then(a["reading"].as_str().cmp(&b["reading"].as_str()))
                    .then(a["surface"].as_str().cmp(&b["surface"].as_str()))
            });
            return Ok(serde_json::json!({"entries": entries}));
        }
        let mut updated = history.clone();
        match command {
            LearningCommand::List => unreachable!(),
            LearningCommand::Delete { reading, surface } => {
                remove_pair(&mut updated, &reading, &surface)
            }
            LearningCommand::Save {
                original_reading,
                original_surface,
                reading,
                surface,
            } => {
                for (text, limit) in [(&reading, 128), (&surface, 256)] {
                    anyhow::ensure!(
                        !text.trim().is_empty()
                            && text.chars().count() <= limit
                            && !text.chars().any(char::is_control),
                        "読み・表記が空、長すぎる、または制御文字を含んでいます"
                    );
                }
                match (original_reading, original_surface) {
                    (Some(r), Some(s)) => {
                        anyhow::ensure!(
                            updated
                                .get(&r)
                                .is_some_and(|entries| entries.iter().any(|e| e.surface == s)),
                            "この履歴は既に削除されています。再読み込みしてください"
                        );
                        if r != reading || s != surface {
                            remove_pair(&mut updated, &r, &s);
                        }
                    }
                    (None, None) => {}
                    _ => anyhow::bail!("invalid original learning entry"),
                }
                let now = now_unix_secs();
                let entries = updated.entry(reading).or_default();
                // Explicit preference beats old frequency immediately. Subsequent
                // commits still use normal frequency/recency learning.
                let best = entries
                    .iter()
                    .map(|e| e.score(now))
                    .fold(now as f64, f64::max);
                let frequency = ((best + LEARN_W_FREQ - now as f64
                    + surface.chars().count() as f64)
                    / LEARN_W_FREQ)
                    .max(1.0) as f32;
                anyhow::ensure!(frequency.is_finite(), "invalid learning frequency");
                entries.retain(|e| e.surface != surface);
                entries.push(LearnEntry {
                    surface,
                    last_access_time: now,
                    suggestion_freq: frequency,
                    shown_freq: 0,
                });
                trim_learn_history_to_capacity(&mut updated, LEARN_LRU_CAPACITY);
            }
        }
        if let Some(path) = &self.inner.learn_history_path {
            save_learn_history_file(path, &updated)?;
        }
        *history = updated;
        Ok(serde_json::json!({"ok": true}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(original: Option<(&str, &str)>, reading: &str, surface: &str) -> LearningCommand {
        LearningCommand::Save {
            original_reading: original.map(|p| p.0.into()),
            original_surface: original.map(|p| p.1.into()),
            reading: reading.into(),
            surface: surface.into(),
        }
    }

    #[test]
    fn edit_promote_delete_and_reload_preserve_other_learning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.bin");
        let store = DictStore::load(None, None, Some(&path)).unwrap();
        for _ in 0..5 {
            store.learn("あるの", "アルノ");
        }
        store.learn("べつ", "ベツ");
        store
            .manage_learning(save(None, "あるの", "あるの"))
            .unwrap();
        assert_eq!(store.lookup_learn("あるの"), ["あるの", "アルノ"]);
        store
            .manage_learning(save(Some(("あるの", "アルノ")), "あるの", "有るの"))
            .unwrap();
        assert_eq!(store.lookup_learn("あるの"), ["有るの", "あるの"]);
        store
            .manage_learning(LearningCommand::Delete {
                reading: "あるの".into(),
                surface: "有るの".into(),
            })
            .unwrap();
        let reloaded = DictStore::load(None, None, Some(&path)).unwrap();
        assert_eq!(reloaded.lookup_learn("あるの"), ["あるの"]);
        assert_eq!(reloaded.lookup_learn("べつ"), ["ベツ"]);
        assert_eq!(
            reloaded.manage_learning(LearningCommand::List).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn invalid_and_stale_edits_do_not_remove_existing_history() {
        let store = DictStore::empty();
        store.learn("あるの", "アルノ");
        for command in [
            save(Some(("あるの", "アルノ")), "", "あるの"),
            save(Some(("あるの", "削除済み")), "あるの", "あるの"),
            save(None, "あるの", "改\n行"),
        ] {
            assert!(store.manage_learning(command).is_err());
            assert_eq!(store.lookup_learn("あるの"), ["アルノ"]);
        }
    }

    #[test]
    fn failed_persistence_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.bin");
        let store = DictStore::load(None, None, Some(&path)).unwrap();
        store.learn("あるの", "アルノ");
        std::fs::create_dir(path.with_extension("bin.tmp")).unwrap();
        assert!(
            store
                .manage_learning(save(None, "あるの", "あるの"))
                .is_err()
        );
        assert_eq!(store.lookup_learn("あるの"), ["アルノ"]);
    }
}
