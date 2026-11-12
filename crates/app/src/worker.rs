//! One pandas process. A job that arrives while a read is in progress kills it.

use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use graph::{Job, JobOp, Reply, write_frame};

struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

pub struct Worker {
    tx: Sender<Job>,
    rx: Receiver<(u64, JobOp, Result<Reply, String>)>,
    pid: Arc<AtomicU32>,
    busy: Arc<AtomicBool>,
}

impl Worker {
    pub fn spawn() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (reply_tx, reply_rx) = mpsc::channel();
        let pid = Arc::new(AtomicU32::new(0));
        let busy = Arc::new(AtomicBool::new(false));
        let pid_thread = pid.clone();
        let busy_thread = busy.clone();
        thread::spawn(move || worker_loop(job_rx, reply_tx, pid_thread, busy_thread));
        Self {
            tx: job_tx,
            rx: reply_rx,
            pid,
            busy,
        }
    }

    pub fn cancel(&self) {
        let pid = self.pid.load(Ordering::SeqCst);
        if pid != 0 {
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
    }

    pub fn submit(&self, job: Job) {
        if self.busy.load(Ordering::SeqCst) {
            let pid = self.pid.load(Ordering::SeqCst);
            if pid != 0 {
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
        }
        let _ = self.tx.send(job);
    }

    /// The next finished job: its generation, its op, and the outcome. A
    /// `JobOp::Reset` reply carries no payload; errors still surface.
    pub fn try_recv(&self) -> Option<(u64, JobOp, Result<Reply, String>)> {
        self.rx.try_recv().ok()
    }
}

fn worker_loop(
    jobs: Receiver<Job>,
    replies: Sender<(u64, JobOp, Result<Reply, String>)>,
    pid: Arc<AtomicU32>,
    busy: Arc<AtomicBool>,
) {
    let mut proc = match start(&pid) {
        Ok(proc) => proc,
        Err(err) => {
            let _ = replies.send((0, JobOp::Preview, Err(err)));
            return;
        }
    };
    while let Ok(job) = jobs.recv() {
        // Drop jobs that were queued behind a newer one. The newest is the last.
        let job = latest(job, &jobs);
        if !alive(&mut proc) {
            proc = match start(&pid) {
                Ok(proc) => proc,
                Err(err) => {
                    let _ = replies.send((job.generation, job.op, Err(err)));
                    continue;
                }
            };
        }
        busy.store(true, Ordering::SeqCst);
        let requested = request(&mut proc, &job);
        busy.store(false, Ordering::SeqCst);
        match requested {
            Ok(reply) => {
                let _ = replies.send((job.generation, job.op, Ok(reply)));
            }
            Err(err) => {
                let _ = replies.send((job.generation, job.op, Err(err)));
                proc = match start(&pid) {
                    Ok(proc) => proc,
                    Err(err) => {
                        let _ = replies.send((job.generation, job.op, Err(err)));
                        continue;
                    }
                };
            }
        }
    }
}

fn latest(mut job: Job, jobs: &Receiver<Job>) -> Job {
    while let Ok(next) = jobs.try_recv() {
        job = next;
    }
    job
}

fn alive(proc: &mut Proc) -> bool {
    matches!(proc.child.try_wait(), Ok(None))
}

fn start(pid: &AtomicU32) -> Result<Proc, String> {
    let mut child = python_command()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("could not start the pandas worker: {err}"))?;
    pid.store(child.id(), Ordering::SeqCst);
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
                eprintln!("worker: {line}");
            }
        });
    }
    let stdin = child.stdin.take().ok_or("worker stdin")?;
    let stdout = child.stdout.take().ok_or("worker stdout")?;
    Ok(Proc {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

fn request(proc: &mut Proc, job: &Job) -> Result<Reply, String> {
    write_frame(&mut proc.stdin, job).map_err(|err| err.to_string())?;
    graph::read_frame(&mut proc.stdout).map_err(|err| err.to_string())
}

pub fn python_command() -> Command {
    let dir = worker_dir();
    let venv = dir.join("../.venv/bin/python");
    let program = if venv.exists() {
        venv
    } else {
        PathBuf::from("python3")
    };
    let mut command = Command::new(program);
    command
        .arg("-m")
        .arg("pipeline")
        .env("PYTHONPATH", &dir)
        .env("PYTHONUNBUFFERED", "1");
    command
}

pub fn worker_dir() -> PathBuf {
    if let Ok(path) = std::env::var("OMPP_WORKER") {
        return PathBuf::from(path);
    }
    let mut candidates = Vec::new();
    candidates.push(PathBuf::from("worker"));
    if let Some(parent) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(|exe| exe.parent())
    {
        candidates.push(parent.join("worker"));
        if let Some(grand) = parent.parent() {
            candidates.push(grand.join("worker"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../worker"));
    candidates
        .into_iter()
        .find(|path| path.join("pipeline").join("__main__.py").is_file())
        .unwrap_or_else(|| PathBuf::from("worker"))
}

/// Used by the CLI. One job, then the process exits.
pub fn run_job(job: &Job) -> Result<Reply, String> {
    let pid = AtomicU32::new(0);
    let mut proc = start(&pid)?;
    let reply = request(&mut proc, job);
    let _ = proc.child.kill();
    reply
}
