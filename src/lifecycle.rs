//! Signal handlers only set a flag. Normal Rust code saves state and reaps children.
use anyhow::Result;
use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
    process::{Child, Command},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

static SHUTDOWN: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static CHILDREN: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
fn children() -> &'static Mutex<HashSet<u32>> {
    CHILDREN.get_or_init(Mutex::default)
}
pub fn requested() -> bool {
    SHUTDOWN.get().is_some_and(|s| s.load(Ordering::Relaxed))
}

pub struct Lifetime;
pub fn install() -> Result<Lifetime> {
    let flag = SHUTDOWN.get_or_init(|| Arc::new(AtomicBool::new(false)));
    #[cfg(unix)]
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, flag.clone())?;
    }
    Ok(Lifetime)
}
fn kill_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        if let Some(flag) = SHUTDOWN.get() {
            flag.store(true, Ordering::Relaxed);
        }
        // Also runs when the main thread unwinds after a terminal or stdout error.
        let owned = children()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect::<Vec<_>>();
        for pid in owned {
            kill_group(pid);
        }
    }
}
pub struct ManagedChild {
    pub child: Child,
    cleaned: bool,
}
impl ManagedChild {
    pub fn kill(&mut self) {
        if self.cleaned {
            return;
        }
        if children()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.child.id())
        {
            kill_group(self.child.id());
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        self.cleaned = true;
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.kill();
    }
}
impl Deref for ManagedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
pub fn spawn_group(command: &mut Command) -> Result<ManagedChild> {
    if requested() {
        anyhow::bail!("Application is shutting down");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    children()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(child.id());
    let mut managed = ManagedChild {
        child,
        cleaned: false,
    };
    if requested() {
        managed.kill();
        anyhow::bail!("Application is shutting down");
    }
    Ok(managed)
}
