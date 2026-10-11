use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub(super) struct State {
    pub host_id: String,
    pub registration: Option<Registration>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Registration {
    pub client_id: String,
    pub subject: Option<String>,
    pub email: Option<String>,
    pub credentials: Option<Credentials>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub expires_at: u64,
    pub scopes: Vec<String>,
}

#[derive(Clone)]
pub(super) struct Storage {
    directory: PathBuf,
}

impl Storage {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn lock(&self) -> Result<File, String> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)
            .map_err(|error| format!("Cannot create Rig configuration directory: {error}"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.directory.join("chatgpt.lock"))
            .map_err(|error| format!("Cannot open ChatGPT login lock: {error}"))?;
        lock.try_lock().map_err(|_| {
            "Another Rig process is updating ChatGPT login; try again when it finishes".to_owned()
        })?;
        Ok(lock)
    }

    pub fn load(&self) -> Result<Option<State>, String> {
        match fs::read(self.path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| "Cannot read ChatGPT login state: invalid JSON".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Cannot read ChatGPT login state: {error}")),
        }
    }

    pub fn save(&self, state: &State) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("Cannot encode ChatGPT login state: {error}"))?;
        let temporary = self.directory.join("chatgpt.json.tmp");
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, self.path())?;
            File::open(&self.directory)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result.map_err(|error: std::io::Error| format!("Cannot save ChatGPT login state: {error}"))
    }

    fn path(&self) -> impl AsRef<Path> {
        self.directory.join("chatgpt.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn saves_private_state_atomically_and_serializes_writers() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::new(directory.path().join("rig"));
        let guard = storage.lock().unwrap();
        assert!(storage.lock().is_err());
        assert!(storage.load().unwrap().is_none());
        let mut state = State {
            host_id: "host".into(),
            registration: None,
        };
        storage.save(&state).unwrap();
        state.host_id = "same-host-updated".into();
        storage.save(&state).unwrap();
        assert_eq!(
            storage.load().unwrap().unwrap().host_id,
            "same-host-updated"
        );
        assert_eq!(
            fs::metadata(storage.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!storage.directory.join("chatgpt.json.tmp").exists());
        drop(guard);
        assert!(storage.lock().is_ok());
    }
}
