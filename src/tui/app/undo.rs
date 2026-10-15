use super::{EffectResult, Runtime};
use sshx::mutation::AppliedChange;
use sshx::usage;
use std::path::PathBuf;

/// Usage state that moved to a new key when a write gave the HostEntry an `##SSHX ID`;
/// undo moves it back and redo forward again.
pub(crate) struct Rekey {
    pub root: PathBuf,
    pub old: String,
    pub new: String,
}

struct Entry {
    label: String,
    change: AppliedChange,
    rekey: Option<Rekey>,
}

/// Applied writes of this process, in memory only (spec: no extra copies of config files on
/// disk). Pair mutations keep their own journal and never land here.
#[derive(Default)]
pub(crate) struct UndoStack {
    done: Vec<Entry>,
    undone: Vec<Entry>,
}

impl UndoStack {
    /// Records a change that was just applied; anything undone before it can no longer be redone.
    pub(crate) fn push(
        &mut self,
        label: impl Into<String>,
        change: AppliedChange,
        rekey: Option<Rekey>,
    ) {
        if change.is_empty() {
            return;
        }
        self.undone.clear();
        self.done.push(Entry {
            label: label.into(),
            change,
            rekey,
        });
    }

    /// Reverts the newest change. A refusal keeps it on the stack.
    pub(crate) fn undo(&mut self) -> Result<String, String> {
        let entry = self
            .done
            .pop()
            .ok_or_else(|| "nothing to undo".to_string())?;
        if let Err(error) = entry.change.undo() {
            self.done.push(entry);
            return Err(error);
        }
        if let Some(rekey) = &entry.rekey {
            let _ = usage::rekey(&rekey.root, &rekey.new, &rekey.old);
        }
        let label = entry.label.clone();
        self.undone.push(entry);
        Ok(label)
    }

    /// Re-applies the newest undone change. A refusal keeps it on the stack.
    pub(crate) fn redo(&mut self) -> Result<String, String> {
        let entry = self
            .undone
            .pop()
            .ok_or_else(|| "nothing to redo".to_string())?;
        if let Err(error) = entry.change.redo() {
            self.undone.push(entry);
            return Err(error);
        }
        if let Some(rekey) = &entry.rekey {
            let _ = usage::rekey(&rekey.root, &rekey.old, &rekey.new);
        }
        let label = entry.label.clone();
        self.done.push(entry);
        Ok(label)
    }
}

fn outcome(result: Result<String, String>, verb: &str) -> EffectResult {
    match result {
        Ok(label) => EffectResult {
            toast: Some(format!("✓ {verb} {label}")),
            refresh: true,
            ..EffectResult::default()
        },
        Err(error) => EffectResult {
            toast: Some(error),
            ..EffectResult::default()
        },
    }
}

pub(super) fn undo(runtime: &mut Runtime) -> Result<EffectResult, String> {
    Ok(outcome(runtime.undo.undo(), "undid"))
}

pub(super) fn redo(runtime: &mut Runtime) -> Result<EffectResult, String> {
    Ok(outcome(runtime.undo.redo(), "redid"))
}

#[cfg(test)]
mod tests {
    use super::{Rekey, UndoStack};
    use sshx::mutation::{self, CreateRequest};
    use sshx::usage;
    use std::fs;
    use std::path::PathBuf;

    struct Fixture {
        directory: PathBuf,
        config: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let directory =
                std::env::temp_dir().join(format!("sshx-undo-{name}-{}", std::process::id()));
            fs::create_dir_all(&directory).unwrap();
            let config = directory.join("config");
            fs::write(&config, "Host a\n  HostName a.example\n").unwrap();
            Self { directory, config }
        }

        fn create(&self, stack: &mut UndoStack, alias: &str) {
            let plan = mutation::plan_create(&CreateRequest::new(
                self.config.clone(),
                self.config.clone(),
                alias.to_string(),
                format!("{alias}.example"),
                None,
                None,
                None,
            ))
            .unwrap();
            let change = plan.change();
            mutation::apply(&plan).unwrap();
            stack.push(format!("create {alias}"), change, None);
        }

        fn text(&self) -> String {
            fs::read_to_string(&self.config).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn undo_and_redo_walk_the_stack_newest_first() {
        let fixture = Fixture::new("walk");
        let original = fixture.text();
        let mut stack = UndoStack::default();
        fixture.create(&mut stack, "b");
        let with_b = fixture.text();
        fixture.create(&mut stack, "c");
        let with_c = fixture.text();

        assert_eq!(stack.undo().unwrap(), "create c");
        assert_eq!(fixture.text(), with_b);
        assert_eq!(stack.undo().unwrap(), "create b");
        assert_eq!(fixture.text(), original);
        assert_eq!(stack.undo().unwrap_err(), "nothing to undo");

        assert_eq!(stack.redo().unwrap(), "create b");
        assert_eq!(stack.redo().unwrap(), "create c");
        assert_eq!(fixture.text(), with_c);
        assert_eq!(stack.redo().unwrap_err(), "nothing to redo");
    }

    #[test]
    fn a_new_change_discards_the_redo_history() {
        let fixture = Fixture::new("discard");
        let mut stack = UndoStack::default();
        fixture.create(&mut stack, "b");
        stack.undo().unwrap();
        fixture.create(&mut stack, "c");
        assert_eq!(stack.redo().unwrap_err(), "nothing to redo");
    }

    #[test]
    fn refused_undo_keeps_the_change_on_the_stack() {
        let fixture = Fixture::new("refused");
        let mut stack = UndoStack::default();
        fixture.create(&mut stack, "b");
        let written = fixture.text();
        fs::write(&fixture.config, format!("{written}# external\n")).unwrap();

        let error = stack.undo().unwrap_err();
        assert!(error.starts_with("UNDO_REFUSED"), "{error}");
        assert_eq!(fixture.text(), format!("{written}# external\n"));

        fs::write(&fixture.config, &written).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.config)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        let error = stack.undo().unwrap_err();
        assert!(error.starts_with("UNDO_REFUSED"), "{error}");
    }

    #[test]
    fn undo_and_redo_move_usage_between_keys() {
        let fixture = Fixture::new("rekey");
        let sidecar = usage::sidecar_path(&fixture.config);
        let keys = || {
            let text = fs::read_to_string(&sidecar).unwrap();
            let file: serde_json::Value = serde_json::from_str(&text).unwrap();
            file["hosts"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        fs::write(
            &sidecar,
            r#"{"version":1,"hosts":{"id-1":{"last_used":5,"count":2,"forwards":[]}}}"#,
        )
        .unwrap();
        let mut stack = UndoStack::default();
        let plan = mutation::plan_create(&CreateRequest::new(
            fixture.config.clone(),
            fixture.config.clone(),
            "b".to_string(),
            "b.example".to_string(),
            None,
            None,
            None,
        ))
        .unwrap();
        let change = plan.change();
        mutation::apply(&plan).unwrap();
        let rekey = Rekey {
            root: fixture.config.clone(),
            old: "a@config".to_string(),
            new: "id-1".to_string(),
        };
        stack.push("edit a", change, Some(rekey));

        stack.undo().unwrap();
        assert_eq!(keys(), ["a@config"]);
        stack.redo().unwrap();
        assert_eq!(keys(), ["id-1"]);
    }
}
