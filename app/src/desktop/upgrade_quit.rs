//! Same-user idle-only GUI quit for a version-bound Desktop upgrade.
//! Reuses private IPC and native exit observation; never sends process signals.
use std::{path::Path, sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use serde_json::Value;
type Error = Box<dyn std::error::Error + Send + Sync>;
pub const CAPABILITY: &str = "gui-upgrade-quit.v1";
pub const GUI_CLOSE_CAPABILITY: &str = "gui-close.v1";
pub struct Operation(Arc<AtomicUsize>);
impl Operation {
    pub fn begin(active: &Arc<AtomicUsize>) -> Self { active.fetch_add(1,Ordering::AcqRel);Self(active.clone()) }
}
impl Drop for Operation {fn drop(&mut self) {self.0.fetch_sub(1,Ordering::AcqRel);}}
pub fn idle(statuses: &[Value], lease: bool, operations: usize) -> Result<(), &'static str> {
    if statuses.is_empty() || lease || operations != 0 {return Err("GUI delivery, receive lease or local command is still active");}
    for status in statuses {
        if status["pending"].as_u64()!=Some(0) || !status["sessions"].as_array().is_some_and(|s|s.is_empty()) {
            return Err("Close active or pending sessions and transfers before upgrading");
        }
    }
    Ok(())
}
pub fn command(directory:&Path,args:&[String]) -> Result<Value,Error> {
    let [_,generation,hash]=args else {return Err("GUI upgrade quit requires expected generation and executable SHA256".into());};
    let user_close = args[0] == "--gui-close-json";
    let (field,capability) = if user_close {("gui_close",GUI_CLOSE_CAPABILITY)} else {("upgrade_quit",CAPABILITY)};
    if crate::background_service::executable_hash()? != *hash {return Err("CLI executable differs from expected GUI owner".into());}
    let status=super::directed_receive::exchange(directory,&super::directed_receive::Request::Status)?;
    if status["ok"]!=true || status["generation"]!=*generation || status["executable_sha256"]!=*hash
        || status[field]!=capability {return Err("Exact GUI owner does not support the requested close operation".into());}
    let pid=status["pid"].as_u64().and_then(|p|u32::try_from(p).ok()).ok_or("GUI PID unavailable")?;
    let observer=crate::background_service::exit_observer::ExitObserver::open(pid)?;
    let id=format!("{}-{}",std::process::id(),crate::unix_now_ms());
    let deadline_ms=crate::unix_now_ms().saturating_add(5000);
    let request = if user_close {
        super::directed_receive::Request::CloseGui {generation:generation.clone(),executable_sha256:hash.clone(),request_id:id.clone(),deadline_ms}
    } else {
        super::directed_receive::Request::UpgradeQuit {generation:generation.clone(),executable_sha256:hash.clone(),request_id:id.clone(),deadline_ms}
    };
    let result=super::directed_receive::exchange(directory,&request)?;
    if result["ok"]!=true || result["cleanup_verified"]!=true || result["generation"]!=*generation
        || result["executable_sha256"]!=*hash || result["request_id"]!=id || result["pid"]!=pid {
        return Err("GUI upgrade quit was rejected or outcome is unconfirmed; observe original owner, do not replay".into());
    }
    observer.wait(Duration::from_secs(5))?;
    if !crate::private_ipc::absent(directory,"gui-receive.sock")? {return Err("GUI endpoint remains or was replaced; stop unconfirmed".into());}
    let lock_path=directory.join("desktop-instance.lock");
    crate::private_ipc::private_file(&lock_path)?;
    let lock=std::fs::OpenOptions::new().read(true).open(lock_path)?;
    lock.try_lock().map_err(|_|"GUI lifecycle lock is still held; stop unconfirmed")?;
    Ok(serde_json::json!({"ok":true,"kind":capability,"generation":generation,"pid":pid,
        "executable_sha256":hash,"cleanup_verified":true,"process_exit_observed":true}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn upgrade_quit_accepted_file_task_blocks_even_before_first_poll_or_serial_lock() {
        let active=Arc::default();let operation=Operation::begin(&active);
        let (release,waiting)=tokio::sync::oneshot::channel::<()>();
        let task=tokio::spawn(async move {let _operation=operation;let _=waiting.await;});
        let empty=serde_json::json!({"pending":0,"sessions":[]});
        assert!(idle(&[empty.clone()],false,active.load(Ordering::Acquire)).is_err());
        tokio::task::yield_now().await;
        assert!(idle(&[empty.clone()],false,active.load(Ordering::Acquire)).is_err());
        release.send(()).unwrap();task.await.unwrap();
        assert!(idle(&[empty],false,active.load(Ordering::Acquire)).is_ok());
    }
    #[test]
    fn upgrade_quit_other_window_or_sender_only_blocks_empty_sessions() {
        let shared=Arc::default();
        let empty=serde_json::json!({"pending":0,"sessions":[]});
        for _kind in ["other-window-file-command","sender-contacting","sender-opening-receiver","other-window-receive-lease"] {
            let work=Operation::begin(&shared);
            assert!(idle(&[empty.clone(),empty.clone()],false,shared.load(Ordering::Acquire)).is_err());
            drop(work);
            assert!(idle(&[empty.clone(),empty.clone()],false,shared.load(Ordering::Acquire)).is_ok());
        }
    }
    #[test]
    fn upgrade_quit_refuses_sessions_pending_leases_and_inflight_commands() {
        let empty=serde_json::json!({"pending":0,"sessions":[]});
        assert!(idle(&[empty.clone()],false,0).is_ok());
        for status in [serde_json::json!({"pending":1,"sessions":[]}),serde_json::json!({"pending":0,"sessions":[{"transfers":1}]}),Value::Null] {
            assert!(idle(&[status],false,0).is_err());
        }
        assert!(idle(&[empty.clone()],true,0).is_err());
        let active=Arc::default();let work=Operation::begin(&active);
        assert!(idle(&[empty.clone()],false,active.load(Ordering::Acquire)).is_err());
        drop(work);assert!(idle(&[empty],false,active.load(Ordering::Acquire)).is_ok());
    }
}
