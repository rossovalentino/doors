use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

pub struct Workspace {
    pub root: PathBuf,
}
impl Workspace {
    pub fn new() -> Self {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self::with_stamp(id)
    }
    fn with_stamp(id: u128) -> Self {
        for _ in 0..128 {
            let serial = NEXT_WORKSPACE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .expect("test workspace counter exhausted");
            let root = std::env::temp_dir().join(format!(
                "door-infrastructure-test-{id}-{}-{serial}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    return Self {
                        root: root.canonicalize().unwrap(),
                    };
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create private test workspace: {error}"),
            }
        }
        panic!("test workspace collision retry bound exhausted")
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        // Constructed exclusively above; neither paths nor lifetime are supplied
        // by test input. Refuse accidental cleanup outside the temporary root.
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        assert!(self.root.starts_with(&temporary));
        assert_ne!(self.root, temporary);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn equal_clock_stamps_in_parallel_have_independent_private_workspaces() {
    let workers: Vec<_> = (0..16)
        .map(|_| std::thread::spawn(|| Workspace::with_stamp(0)))
        .collect();
    let workspaces: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    let paths: std::collections::BTreeSet<_> =
        workspaces.iter().map(|workspace| &workspace.root).collect();
    assert_eq!(paths.len(), 16);
    for workspace in &workspaces {
        assert!(workspace.root.is_dir());
    }
}
