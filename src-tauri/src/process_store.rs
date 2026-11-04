//! Owned child-process store. All launched runners are tracked by their OS PID
//! and a retained Child handle. Stop only acts on owned children.
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;

pub struct ProcessEntry {
    pub child: Child,
    pub executable_path: PathBuf,
}

pub struct ProcessStore {
    processes: HashMap<u32, ProcessEntry>,
}

impl ProcessStore {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    /// Reap naturally exited children and return currently live PIDs.
    pub fn running_pids(&mut self) -> Vec<u32> {
        let mut dead: Vec<u32> = Vec::new();
        let mut live: Vec<u32> = Vec::new();
        for (&pid, entry) in &mut self.processes {
            match entry.child.try_wait() {
                Ok(Some(_)) => dead.push(pid),
                Ok(None) => live.push(pid),
                // An observation failure is not proof that the child exited.
                Err(_) => live.push(pid),
            }
        }
        for pid in dead {
            self.processes.remove(&pid);
        }
        live.sort_unstable();
        live
    }

    /// True if a live process is already running for the given executable path.
    pub fn has_running_at(&mut self, executable_path: &PathBuf) -> bool {
        self.running_pids();
        self.processes
            .values()
            .any(|e| e.executable_path == *executable_path)
    }

    /// Insert a new owned child. Rejects duplicate active launch for the same
    /// executable path.
    pub fn insert(&mut self, mut child: Child, executable_path: PathBuf) -> Result<u32, String> {
        if self.has_running_at(&executable_path) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "process already running for {:?}",
                executable_path
            ));
        }
        let pid = child.id();
        self.processes
            .insert(pid, ProcessEntry { child, executable_path });
        Ok(pid)
    }

    /// Stop an owned child by PID. Returns true if the child was found and
    /// stopped, false if it was already reaped.
    pub fn stop(&mut self, pid: u32) -> Result<bool, String> {
        if let Some(mut entry) = self.processes.remove(&pid) {
            match entry.child.kill() {
                Ok(_) => {
                    let _ = entry.child.wait();
                    Ok(true)
                }
                Err(e) => {
                    self.processes.insert(pid, entry);
                    Err(format!("failed to kill process {}: {}", pid, e))
                }
            }
        } else {
            Ok(false)
        }
    }

    /// Kill all tracked children. Used on application exit.
    pub fn cleanup(&mut self) {
        self.running_pids();
        for (_, mut entry) in self.processes.drain() {
            let _ = entry.child.kill();
            let _ = entry.child.wait();
        }
    }
}

impl Default for ProcessStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn sleep_child() -> Child {
        #[cfg(windows)]
        let mut cmd = {
            use std::os::windows::process::CommandExt;
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 30"]);
            command.creation_flags(0x08000000);
            command
        };
        #[cfg(not(windows))]
        let mut cmd = Command::new("sleep");
        #[cfg(not(windows))]
        cmd.arg("10");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn insert_returns_pid() {
        let mut store = ProcessStore::new();
        let child = sleep_child();
        let pid = child.id();
        let path = PathBuf::from("/games/123/game.exe");
        let returned = store.insert(child, path.clone()).unwrap();
        assert_eq!(returned, pid);
        let _ = store.stop(pid);
    }

    #[test]
    fn rejects_duplicate_path() {
        let mut store = ProcessStore::new();
        let child1 = sleep_child();
        let child2 = sleep_child();
        let path = PathBuf::from("/games/123/game.exe");
        let pid1 = child1.id();
        let pid2 = child2.id();
        store.insert(child1, path.clone()).unwrap();
        assert!(store.insert(child2, path).is_err());
        assert_eq!(store.running_pids(), vec![pid1]);
        assert!(!store.stop(pid2).unwrap());
        store.stop(pid1).unwrap();
    }

    #[test]
    fn stop_rejects_unowned_pid() {
        let mut store = ProcessStore::new();
        let result = store.stop(999_999);
        assert_eq!(result.unwrap(), false);
    }

    #[test]
    fn same_image_name_isolation() {
        let mut store = ProcessStore::new();
        let child1 = sleep_child();
        let mut unrelated = sleep_child();
        let pid1 = child1.id();
        let unrelated_pid = unrelated.id();
        store.insert(child1, PathBuf::from("/games/123/same.exe")).unwrap();
        assert!(!store.stop(unrelated_pid).unwrap());
        assert!(store.stop(pid1).unwrap());
        assert!(unrelated.try_wait().unwrap().is_none(), "unowned same-image child was terminated");
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }
}
