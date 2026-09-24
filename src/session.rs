use anyhow::{Context, Result, bail};
use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub role: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Check {
    pub path: String,
    pub kind: String,
    pub expected: Value,
    pub passed: bool,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Steer,
    FollowUp,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingMessage {
    pub id: String,
    pub text: String,
    pub delivery: Delivery,
}
impl PendingMessage {
    pub fn new(text: String, delivery: Delivery) -> Self {
        Self {
            id: Uuid::new_v4().simple().to_string()[..12].into(),
            text,
            delivery,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub project: PathBuf,
    pub created: String,
    pub updated: String,
    pub model: String,
    pub demo: bool,
    pub mode: String,
    pub status: String,
    pub entries: Vec<Entry>,
    pub messages: Vec<Value>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tools: u64,
    pub checks: Vec<Check>,
    pub parent: Option<String>,
    #[serde(default)]
    pub work: crate::work::Work,
    #[serde(default)]
    pub pending: Vec<PendingMessage>,
}
impl Session {
    pub fn new(project: PathBuf, model: String, demo: bool) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            version: 2,
            id: Uuid::new_v4().simple().to_string()[..12].into(),
            title: "A fresh conversation".into(),
            project,
            created: now.clone(),
            updated: now,
            model,
            demo,
            mode: "build".into(),
            status: "idle".into(),
            entries: vec![],
            messages: vec![],
            input_tokens: 0,
            output_tokens: 0,
            tools: 0,
            checks: vec![],
            parent: None,
            work: Default::default(),
            pending: vec![],
        }
    }
    pub fn add(&mut self, role: &str, text: impl Into<String>) {
        self.entries.push(Entry {
            role: role.into(),
            text: text.into(),
        });
        self.updated = Utc::now().to_rfc3339();
    }
    pub fn fork(&self) -> Self {
        let mut s = self.clone();
        s.id = Uuid::new_v4().simple().to_string()[..12].into();
        s.parent = Some(self.id.clone());
        s.title = format!("{} · fork", self.title);
        s.created = Utc::now().to_rfc3339();
        s.updated = s.created.clone();
        s.status = "idle".into();
        s.pending.clear();
        s
    }
}
pub struct Store {
    pub root: PathBuf,
    _lock: File,
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(".lock"))?;
        lock.try_lock_exclusive().context(
            "Another Aster is using this session store. Use --state-dir for a separate store.",
        )?;
        Ok(Self {
            root: root.into(),
            _lock: lock,
        })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 12 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Invalid session ID")
        };
        Ok(self.root.join(format!("{id}.json")))
    }
    pub fn save(&self, s: &Session) -> Result<()> {
        atomic_json(&self.path(&s.id)?, s)
    }
    pub fn load(&self, id: &str) -> Result<Session> {
        Ok(serde_json::from_slice(&fs::read(self.path(id)?)?)?)
    }
    pub fn list(&self, project: &Path) -> Result<Vec<Session>> {
        let mut list = vec![];
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().is_some_and(|x| x == "json")
                && let Ok(s) = serde_json::from_slice::<Session>(&fs::read(path)?)
                && s.project == project
            {
                list.push(s);
            }
        }
        list.sort_by(|a, b| b.updated.cmp(&a.updated));
        Ok(list)
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        fs::remove_file(self.path(id)?)?;
        Ok(())
    }
    pub fn export(&self, s: &Session) -> Result<PathBuf> {
        let dir = self.root.join("exports");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "{}-{}.md",
            s.id,
            Utc::now().format("%Y%m%d-%H%M%S")
        ));
        let mut text = format!("# {}\n\nProject: {}\n\n", s.title, s.project.display());
        for e in &s.entries {
            text += &format!("## {}\n\n{}\n\n", e.role, e.text);
        }
        private_write(&path, text.as_bytes())?;
        Ok(path)
    }
}
pub fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}
pub fn atomic_json(path: &Path, data: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
    private_write(&tmp, &serde_json::to_vec_pretty(data)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_sessions_default_queue_and_forks_do_not_duplicate_pending_work() {
        let mut s = Session::new(PathBuf::from("/project"), "test".into(), true);
        s.pending
            .push(PendingMessage::new("next".into(), Delivery::FollowUp));
        assert!(s.fork().pending.is_empty());
        assert_eq!(s.pending.len(), 1);
        let mut old = serde_json::to_value(&s).unwrap();
        old.as_object_mut().unwrap().remove("pending");
        old.as_object_mut().unwrap().remove("work");
        let restored: Session = serde_json::from_value(old).unwrap();
        assert!(restored.pending.is_empty());
        assert!(restored.work.goal.is_empty());
    }
    #[test]
    fn persist_fork_export_and_lock() {
        let d = tempfile::tempdir().unwrap();
        let st = Store::open(d.path()).unwrap();
        assert!(Store::open(d.path()).is_err());
        let mut s = Session::new(d.path().into(), "test".into(), true);
        s.add("you", "hello");
        s.messages.push(serde_json::json!({"private":"thinking"}));
        st.save(&s).unwrap();
        assert_eq!(st.load(&s.id).unwrap().entries[0].text, "hello");
        let b = s.fork();
        assert_ne!(b.id, s.id);
        assert_eq!(b.parent, Some(s.id));
        assert!(
            !fs::read_to_string(st.export(&b).unwrap())
                .unwrap()
                .contains("thinking")
        );
        assert!(st.load("../../bad").is_err());
    }
}
