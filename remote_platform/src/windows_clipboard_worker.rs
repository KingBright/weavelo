//! Keep native clipboard heap faults outside the network and media owners.
//! One persistent, pipe-only worker is shared by this process's connections.
use super::windows_clipboard::NativeClipboardProvider;
use async_trait::async_trait;
use bincode::Options;
use protocol::ClipboardBundle;
use remote_core::{ClipboardBackendCapabilities, ClipboardFileReference, ClipboardFileReferenceProvider, ClipboardProvider, PlatformKind};
use remote_core::clipboard_plane::ClipboardSyncPolicy;
use serde::{Deserialize, Serialize};
use std::{error::Error, io::{self, Read, Write}, path::PathBuf, process::{Child, ChildStdin, ChildStdout, Command, Stdio}, sync::{Mutex, OnceLock}};
use std::os::windows::process::CommandExt;

type Failure = Box<dyn Error + Send + Sync>;
const LIMIT: u64 = 512 * 1024 * 1024 + 4096;

#[derive(Serialize, Deserialize)]
enum Request {
    Read(ClipboardSyncPolicy),
    Write(ClipboardBundle, ClipboardSyncPolicy),
    ReadFiles,
    WriteFiles(Vec<PathBuf>),
}
impl Request {
    fn empty_read(&self) -> Option<Reply> {
        match self { Self::Read(_) => Some(Reply::Bundle(None)), Self::ReadFiles => Some(Reply::Files(Vec::new())), _ => None }
    }
}
#[derive(Serialize, Deserialize)]
enum Reply { Ready, Bundle(Option<ClipboardBundle>), Files(Vec<PathBuf>), Written, Error(String) }

fn read_frame<T: serde::de::DeserializeOwned>(input: &mut impl Read) -> Result<Option<T>, Failure> {
    let mut size = [0u8; 4];
    if input.read(&mut size[..1])? == 0 { return Ok(None); }
    input.read_exact(&mut size[1..])?;
    let length = u32::from_be_bytes(size) as u64;
    if length == 0 || length > LIMIT { return Err("Invalid native clipboard frame size".into()); }
    let mut raw = vec![0u8; length as usize];
    input.read_exact(&mut raw)?;
    Ok(Some(bincode::DefaultOptions::new().with_fixint_encoding().with_limit(LIMIT).reject_trailing_bytes().deserialize(&raw)?))
}
fn write_frame<T: Serialize>(output: &mut impl Write, value: &T) -> Result<(), Failure> {
    let raw = bincode::DefaultOptions::new().with_fixint_encoding().with_limit(LIMIT).serialize(value)?;
    output.write_all(&u32::try_from(raw.len())?.to_be_bytes())?;
    output.write_all(&raw)?;
    output.flush()?;
    Ok(())
}

struct Worker { child: Child, input: ChildStdin, output: ChildStdout }
impl Worker {
    fn start() -> Result<Self, Failure> {
        let mut command = Command::new(std::env::current_exe()?.canonicalize()?);
        command.arg("--native-clipboard-worker");
        Self::spawn(&mut command)
    }
    fn spawn(command: &mut Command) -> Result<Self, Failure> {
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .creation_flags(0x0800_0000).spawn()?;
        let input = child.stdin.take().ok_or("Native clipboard input unavailable")?;
        let output = child.stdout.take().ok_or("Native clipboard output unavailable")?;
        let mut worker = Self { child, input, output };
        if !matches!(read_frame(&mut worker.output)?, Some(Reply::Ready)) {
            return Err("Native clipboard worker did not start".into());
        }
        Ok(worker)
    }
    fn exchange(&mut self, request: &Request) -> Result<Reply, Failure> {
        write_frame(&mut self.input, request)?;
        read_frame(&mut self.output)?.ok_or_else(|| "Native clipboard worker ended".into())
    }
}
impl Drop for Worker {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

#[derive(Default)]
struct State { worker: Option<Worker>, failed_revision: Option<u32> }
impl State {
    fn exchange(&mut self, request: Request) -> Result<Reply, Failure> {
        let revision = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
        if self.failed_revision == Some(revision) {
            if let Some(empty) = request.empty_read() { return Ok(empty); }
        }
        // An idle child may have exited before this new clipboard item existed.
        // Restart it without attributing that old exit to the new item.
        if let Some(worker) = &mut self.worker {
            if worker.child.try_wait()?.is_some() { self.worker.take(); }
        }
        if self.worker.is_none() { self.worker = Some(Worker::start()?); }
        match self.worker.as_mut().unwrap().exchange(&request) {
            Ok(reply) => { self.failed_revision = None; Ok(reply) }
            Err(error) => {
                self.worker.take();
                self.failed_revision = Some(revision);
                eprintln!("Native clipboard operation ended: {error}; waiting for a clipboard change");
                request.empty_read().ok_or(error)
            }
        }
    }
}
async fn exchange(request: Request) -> Result<Reply, Failure> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    tokio::task::spawn_blocking(move || {
        STATE.get_or_init(Default::default).lock().map_err(|_| "Native clipboard state unavailable")?.exchange(request)
    }).await?
}
fn unexpected(reply: Reply) -> Failure {
    match reply { Reply::Error(error) => error.into(), _ => "Unexpected native clipboard reply".into() }
}

#[derive(Default)]
pub struct WindowsClipboardProvider;
impl WindowsClipboardProvider { pub fn new() -> Self { Self } }
#[async_trait]
impl ClipboardProvider for WindowsClipboardProvider {
    fn platform(&self) -> PlatformKind { PlatformKind::Windows }
    fn capabilities(&self) -> ClipboardBackendCapabilities { NativeClipboardProvider::new().capabilities() }
    fn clipboard_revision(&self) -> Option<u64> {
        let revision = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
        (revision != 0).then_some(revision as u64)
    }
    async fn read_clipboard(&mut self, policy: ClipboardSyncPolicy) -> Result<Option<ClipboardBundle>, Failure> {
        match exchange(Request::Read(policy)).await? { Reply::Bundle(bundle) => Ok(bundle), reply => Err(unexpected(reply)) }
    }
    async fn write_clipboard(&mut self, bundle: &ClipboardBundle, policy: ClipboardSyncPolicy) -> Result<(), Failure> {
        match exchange(Request::Write(bundle.clone(), policy)).await? { Reply::Written => Ok(()), reply => Err(unexpected(reply)) }
    }
}
#[async_trait]
impl ClipboardFileReferenceProvider for WindowsClipboardProvider {
    fn platform(&self) -> PlatformKind { PlatformKind::Windows }
    async fn read_clipboard_file_references(&mut self) -> Result<Vec<ClipboardFileReference>, Failure> {
        match exchange(Request::ReadFiles).await? {
            Reply::Files(paths) => Ok(paths.into_iter().map(ClipboardFileReference::new).collect()), reply => Err(unexpected(reply))
        }
    }
    async fn write_clipboard_file_references(&mut self, references: &[ClipboardFileReference]) -> Result<(), Failure> {
        match exchange(Request::WriteFiles(references.iter().map(|r|r.path.clone()).collect())).await? {
            Reply::Written => Ok(()), reply => Err(unexpected(reply))
        }
    }
}

pub async fn run_windows_clipboard_worker() -> Result<(), Failure> {
    use windows_sys::Win32::{Storage::FileSystem::{GetFileType, FILE_TYPE_PIPE}, System::Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE}};
    if unsafe { GetFileType(GetStdHandle(STD_INPUT_HANDLE)) } != FILE_TYPE_PIPE
        || unsafe { GetFileType(GetStdHandle(STD_OUTPUT_HANDLE)) } != FILE_TYPE_PIPE {
        return Err("Native clipboard worker requires its parent pipes".into());
    }
    let mut input = io::stdin().lock(); let mut output = io::stdout().lock();
    write_frame(&mut output, &Reply::Ready)?;
    let mut native = NativeClipboardProvider::new();
    while let Some(request) = read_frame::<Request>(&mut input)? {
        let reply: Result<Reply, Failure> = match request {
            Request::Read(policy) => native.read_clipboard(policy).await.map(Reply::Bundle),
            Request::Write(bundle, policy) => native.write_clipboard(&bundle, policy).await.map(|_| Reply::Written),
            Request::ReadFiles => native.read_clipboard_file_references().await.map(|refs|Reply::Files(refs.into_iter().map(|r|r.path).collect())),
            Request::WriteFiles(paths) => native.write_clipboard_file_references(&paths.into_iter().map(ClipboardFileReference::new).collect::<Vec<_>>()).await.map(|_| Reply::Written),
        };
        write_frame(&mut output, &reply.unwrap_or_else(|error|Reply::Error(error.to_string())))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_frames_reject_partial_oversized_and_trailing_data() {
        assert!(read_frame::<Reply>(&mut &b"\0\0"[..]).is_err());
        assert!(read_frame::<Reply>(&mut &(u32::MAX.to_be_bytes())[..]).is_err());
        let mut frame=Vec::new();write_frame(&mut frame,&Reply::Written).unwrap();
        assert!(matches!(read_frame(&mut &frame[..]).unwrap(),Some(Reply::Written)));
        frame.push(0);let size=(frame.len()-4) as u32;frame[..4].copy_from_slice(&size.to_be_bytes());
        assert!(read_frame::<Reply>(&mut &frame[..]).is_err());
    }
    #[test]
    fn native_child_exit_is_an_error_without_poisoning_the_owner() {
        let mut command=Command::new("cmd.exe");command.args(["/d","/c","exit 11"]);
        assert!(Worker::spawn(&mut command).is_err());
    }
    #[test]
    fn native_fault_after_start_releases_the_worker_and_keeps_the_parent_available() {
        // Pipe-only fixture: it acknowledges startup, then exits during a read.
        // It never reads or writes the OS clipboard.
        let mut command=Command::new("powershell.exe");
        command.args(["-NoProfile","-Command", "$out=[Console]::OpenStandardOutput();$out.Write([byte[]](0,0,0,4,0,0,0,0),0,8);$out.Flush();$null=[Console]::OpenStandardInput().ReadByte();exit 11"]);
        let worker=Worker::spawn(&mut command).unwrap();
        let mut state=State { worker:Some(worker), failed_revision:None };
        assert!(matches!(state.exchange(Request::Read(ClipboardSyncPolicy::default())).unwrap(),Reply::Bundle(None)));
        assert!(state.worker.is_none());
        assert!(state.failed_revision.is_some());
    }
}
