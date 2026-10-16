use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

static EPOCH: AtomicU64 = AtomicU64::new(1);
const PREFIX: &str = "TURNINATOR_EVENT ";

pub fn renderer_script(name: &str) -> PathBuf {
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let bundled = parent.join("renderer").join(name);
        if bundled.is_file() {
            return bundled;
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("renderer")
        .join(name)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolPaths {
    pub blender: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}
impl Default for ToolPaths {
    fn default() -> Self {
        Self {
            blender: "C:/Program Files/Blender Foundation/Blender 4.5/blender.exe".into(),
            ffmpeg: "C:/Program Files/ffmpeg/bin/ffmpeg.exe".into(),
            ffprobe: "C:/Program Files/ffmpeg/bin/ffprobe.exe".into(),
        }
    }
}
impl ToolPaths {
    /// Validate every configured tool. Operations should use `validate_requirements`.
    pub fn validate(&self) -> Result<()> {
        for (label, path) in [
            ("Blender", &self.blender),
            ("FFmpeg", &self.ffmpeg),
            ("ffprobe", &self.ffprobe),
        ] {
            if !path.is_absolute() || !path.is_file() {
                return Err(format!("{label} executable not found at {}", path.display()).into());
            }
        }
        Ok(())
    }
}

/// Object-safe worker transport. The adapter owns this transport on its actor;
/// the production implementation retains WorkerClient's owned-process cancellation.
pub trait WorkerTransport {
    fn request(
        &mut self,
        request: Value,
        cancel: &AtomicBool,
        on_event: &mut dyn FnMut(Value),
    ) -> Result<Value>;
    fn cancel(&mut self);
}
impl WorkerTransport for WorkerClient {
    fn request(
        &mut self,
        request: Value,
        cancel: &AtomicBool,
        on_event: &mut dyn FnMut(Value),
    ) -> Result<Value> {
        WorkerClient::request(self, request, cancel, on_event)
    }
    fn cancel(&mut self) {
        WorkerClient::cancel(self);
    }
}

pub fn quote_windows_argument(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            result.push_str(&"\\".repeat(slashes * 2 + 1));
        } else {
            result.push_str(&"\\".repeat(slashes));
        }
        slashes = 0;
        result.push(c);
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}

pub fn decode_event(raw: &str) -> Result<Value> {
    let event: Value = serde_json::from_str(raw)?;
    if event["protocol_version"] != 1
        || !event["scene_revision"].is_u64()
        || !event["payload"].is_object()
        || !(event["request_id"].is_null() || event["request_id"].is_string())
        || !matches!(
            event["kind"].as_str(),
            Some("ready" | "probe" | "frame" | "complete" | "error" | "shutdown")
        )
    {
        return Err("invalid worker event envelope".into());
    }
    Ok(event)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        fs::File,
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
    };
    use windows_sys::Win32::{
        Foundation::*,
        Security::SECURITY_ATTRIBUTES,
        System::{JobObjects::*, Pipes::CreatePipe, Threading::*},
    };
    struct Handle(usize);
    impl Handle {
        fn raw(&self) -> HANDLE {
            self.0 as HANDLE
        }
        fn file(self) -> File {
            let raw = self.raw();
            std::mem::forget(self);
            unsafe { File::from_raw_handle(raw) }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                if self.0 != 0 {
                    CloseHandle(self.raw());
                }
            }
        }
    }
    pub struct Process {
        process: Handle,
        job: Handle,
    }
    impl Process {
        pub fn spawn(
            executable: &Path,
            args: &[String],
            cwd: Option<&Path>,
        ) -> Result<(Self, File, File)> {
            unsafe {
                let sa = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: std::ptr::null_mut(),
                    bInheritHandle: 1,
                };
                let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
                if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let child_in = Handle(read as usize);
                let parent_in = Handle(write as usize);
                if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let parent_out = Handle(read as usize);
                let child_out = Handle(write as usize);
                if SetHandleInformation(parent_in.raw(), HANDLE_FLAG_INHERIT, 0) == 0
                    || SetHandleInformation(parent_out.raw(), HANDLE_FLAG_INHERIT, 0) == 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                let job = Handle(CreateJobObjectW(std::ptr::null(), std::ptr::null()) as usize);
                if job.0 == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.raw(),
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                let mut command = quote_windows_argument(&executable.to_string_lossy());
                for arg in args {
                    command.push(' ');
                    command.push_str(&quote_windows_argument(arg));
                }
                if command.contains('\0') {
                    return Err("process argument contains NUL".into());
                }
                let mut command: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
                let exe: Vec<u16> = executable
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect();
                let directory = cwd.map(|p| {
                    p.as_os_str()
                        .encode_wide()
                        .chain(Some(0))
                        .collect::<Vec<_>>()
                });
                let mut startup: STARTUPINFOW = std::mem::zeroed();
                startup.cb = std::mem::size_of_val(&startup) as u32;
                startup.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
                startup.hStdInput = child_in.raw();
                startup.hStdOutput = child_out.raw();
                startup.hStdError = child_out.raw();
                let mut info: PROCESS_INFORMATION = std::mem::zeroed();
                if CreateProcessW(
                    exe.as_ptr(),
                    command.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW,
                    std::ptr::null(),
                    directory.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                    &startup,
                    &mut info,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                let process = Handle(info.hProcess as usize);
                let primary_thread = Handle(info.hThread as usize);
                if AssignProcessToJobObject(job.raw(), process.raw()) == 0 {
                    let error = std::io::Error::last_os_error();
                    TerminateProcess(process.raw(), 1);
                    WaitForSingleObject(process.raw(), 10000);
                    return Err(error.into());
                }
                if ResumeThread(primary_thread.raw()) == u32::MAX {
                    let error = std::io::Error::last_os_error();
                    TerminateJobObject(job.raw(), 1);
                    WaitForSingleObject(process.raw(), 10000);
                    return Err(error.into());
                }
                Ok((Self { process, job }, parent_in.file(), parent_out.file()))
            }
        }
        pub fn status(&self) -> Result<Option<u32>> {
            unsafe {
                let mut code = 0;
                if GetExitCodeProcess(self.process.raw(), &mut code) == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                Ok(if code == 259 { None } else { Some(code) })
            }
        }
        pub fn kill(&mut self) {
            unsafe {
                TerminateJobObject(self.job.raw(), 1);
                WaitForSingleObject(self.process.raw(), 10000);
            }
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            if self.status().ok().flatten().is_none() {
                self.kill();
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub struct Process(std::process::Child);
    impl Process {
        pub fn spawn(
            exe: &Path,
            args: &[String],
            cwd: Option<&Path>,
        ) -> Result<(Self, std::process::ChildStdin, std::process::ChildStdout)> {
            let mut c = std::process::Command::new(exe);
            c.args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit());
            if let Some(dir) = cwd {
                c.current_dir(dir);
            }
            let mut child = c.spawn()?;
            let input = child.stdin.take().unwrap();
            let output = child.stdout.take().unwrap();
            Ok((Self(child), input, output))
        }
        pub fn status(&mut self) -> Result<Option<u32>> {
            Ok(self.0.try_wait()?.map(|s| s.code().unwrap_or(1) as u32))
        }
        pub fn kill(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

enum Received {
    Event(u64, Value),
    Log(String),
    Failure(String),
    Eof,
}
pub struct WorkerClient {
    process: platform::Process,
    input: Box<dyn Write + Send>,
    receiver: mpsc::Receiver<Received>,
    reader: Option<thread::JoinHandle<()>>,
    epoch: u64,
    stopped: bool,
}
impl WorkerClient {
    pub fn spawn(tools: &ToolPaths) -> Result<Self> {
        super::requirements::validate_requirements(
            tools,
            &super::requirements::ToolRequirements {
                blender: true,
                ..Default::default()
            },
        )?;
        let script = renderer_script("worker.py");
        let args = [
            "--background",
            "--factory-startup",
            "--disable-autoexec",
            "--python",
        ]
        .map(String::from)
        .into_iter()
        .chain(Some(script.to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
        let (process, input, output) = platform::Process::spawn(&tools.blender, &args, None)?;
        let (sender, receiver) = mpsc::channel();
        let epoch = EPOCH.fetch_add(1, Ordering::Relaxed);
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let read = reader.read_until(b'\n', &mut bytes);
                match read {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) => {
                        let _ = sender.send(Received::Failure(e.to_string()));
                        break;
                    }
                }
                if bytes.len() > 1_048_576 {
                    let _ = sender.send(Received::Failure("worker line exceeds 1 MiB".into()));
                    break;
                }
                let line = String::from_utf8_lossy(&bytes).into_owned();
                let message = if let Some(raw) = line.strip_prefix(PREFIX) {
                    match decode_event(raw) {
                        Ok(v) => Received::Event(epoch, v),
                        Err(e) => Received::Failure(e.to_string()),
                    }
                } else {
                    Received::Log(line)
                };
                if sender.send(message).is_err() {
                    return;
                }
            }
            let _ = sender.send(Received::Eof);
        });
        Ok(Self {
            process,
            input: Box::new(input),
            receiver,
            reader: Some(reader),
            epoch,
            stopped: false,
        })
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn is_running(&mut self) -> bool {
        !self.stopped && self.process.status().ok().flatten().is_none()
    }
    pub fn cancel(&mut self) {
        self.stopped = true;
        self.process.kill();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
    pub fn request(
        &mut self,
        mut request: Value,
        cancel: &AtomicBool,
        mut on_event: impl FnMut(Value),
    ) -> Result<Value> {
        if self.stopped {
            return Err("worker stopped; start a new worker".into());
        }
        if cancel.load(Ordering::Acquire) {
            self.cancel();
            return Err("cancelled".into());
        }
        request["protocol_version"] = json!(1);
        let id = request["request_id"]
            .as_str()
            .ok_or("request_id required")?
            .to_owned();
        let revision = request["scene_revision"]
            .as_u64()
            .ok_or("scene_revision required")?;
        let kind = request["kind"].as_str().ok_or("kind required")?.to_owned();
        let timeout = request["timeout_seconds"]
            .as_u64()
            .unwrap_or(match kind.as_str() {
                "probe" => 60,
                "shutdown" => 20,
                _ => 7200,
            });
        let deadline = Instant::now() + Duration::from_secs(timeout.clamp(1, 86400));
        writeln!(self.input, "{}", serde_json::to_string(&request)?)?;
        self.input.flush()?;
        loop {
            if cancel.load(Ordering::Acquire) {
                self.cancel();
                return Err("cancelled".into());
            }
            if Instant::now() >= deadline {
                self.cancel();
                return Err(format!("worker request {id} timed out").into());
            }
            match self.receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Received::Event(epoch, mut event)) => {
                    if epoch != self.epoch || event["request_id"].as_str() != Some(&id) {
                        continue;
                    }
                    if event["scene_revision"].as_u64() != Some(revision) {
                        self.cancel();
                        return Err("worker scene revision mismatch".into());
                    }
                    event["worker_epoch"] = json!(epoch);
                    if event["kind"] == "error" {
                        return Err(format!("Blender: {}", event["payload"]).into());
                    }
                    on_event(event.clone());
                    if matches!(
                        (kind.as_str(), event["kind"].as_str()),
                        ("probe", Some("probe"))
                            | ("render", Some("complete"))
                            | ("shutdown", Some("shutdown"))
                    ) {
                        return Ok(event);
                    }
                }
                Ok(Received::Log(line)) => on_event(
                    json!({"kind":"log","worker_epoch":self.epoch,"payload":{"message":line}}),
                ),
                Ok(Received::Failure(error)) => {
                    self.cancel();
                    return Err(format!("worker transport: {error}").into());
                }
                Ok(Received::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stopped = true;
                    return Err("Blender worker exited unexpectedly".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
impl Drop for WorkerClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Hidden, owned process tree for encoders and ffprobe. Cancellation is polled
/// while the pipe is drained independently, including blocking native work.
pub fn run_owned(
    executable: &Path,
    args: &[String],
    cwd: Option<&Path>,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<u8>> {
    if cancel.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    let (mut process, input, mut output) = platform::Process::spawn(executable, args, cwd)?;
    drop(input);
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = Read::by_ref(&mut output)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes);
        (result, bytes)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            process.kill();
            break Err("cancelled".to_string());
        }
        if Instant::now() >= deadline {
            process.kill();
            break Err("external tool timed out".to_string());
        }
        match process.status() {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                process.kill();
                break Err(e.to_string());
            }
        }
    };
    let (read_result, bytes) = reader.join().map_err(|_| "tool output reader failed")?;
    read_result?;
    let code = status.map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("tool output exceeded 16 MiB".into());
    }
    if code != 0 {
        return Err(format!(
            "{} exited {code}: {}",
            executable.display(),
            String::from_utf8_lossy(&bytes)
        )
        .into());
    }
    Ok(bytes)
}
