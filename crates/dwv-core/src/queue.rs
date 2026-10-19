use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::db::{Database, State};
use crate::engine::{Download, DownloadController, DownloadSpec, Event, Outcome};
use crate::error::Error;
use crate::limiter::SpeedLimiter;

#[derive(Clone, Debug)]
pub struct QueueConfig {
    pub max_parallel: usize,
    pub speed_limit: Option<u64>,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_parallel: 3,
            speed_limit: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum QueueEvent {
    Started {
        id: u64,
        path: PathBuf,
        total: Option<u64>,
        connections: usize,
        resumed: bool,
    },
    Progress {
        id: u64,
        downloaded: u64,
        total: Option<u64>,
        speed_bps: u64,
    },
    Finished {
        id: u64,
        path: PathBuf,
    },
    Failed {
        id: u64,
        error: String,
    },
    Cancelled {
        id: u64,
    },
    Paused {
        id: u64,
    },
}

enum Command {
    Add {
        spec: DownloadSpec,
        reply: oneshot::Sender<u64>,
    },
    Cancel(u64),
    Pause(u64),
    Resume(u64),
    SetMaxParallel(usize),
    SetSpeedLimit(Option<u64>),
}

#[derive(Clone)]
pub struct Queue {
    cmd_tx: mpsc::UnboundedSender<Command>,
}

impl Queue {
    pub fn start(config: QueueConfig, db: Option<Database>) -> (Queue, mpsc::Receiver<QueueEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::channel(256);
        tokio::spawn(supervisor(config, cmd_rx, ev_tx, db));
        (Queue { cmd_tx }, ev_rx)
    }

    pub async fn add(&self, spec: DownloadSpec) -> Result<u64, Error> {
        let (reply, rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::Add { spec, reply })
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())
    }

    pub fn cancel(&self, id: u64) {
        let _ = self.cmd_tx.send(Command::Cancel(id));
    }

    pub fn pause(&self, id: u64) {
        let _ = self.cmd_tx.send(Command::Pause(id));
    }

    pub fn resume(&self, id: u64) {
        let _ = self.cmd_tx.send(Command::Resume(id));
    }

    pub fn set_max_parallel(&self, max: usize) {
        let _ = self.cmd_tx.send(Command::SetMaxParallel(max.max(1)));
    }

    pub fn set_speed_limit(&self, limit: Option<u64>) {
        let _ = self
            .cmd_tx
            .send(Command::SetSpeedLimit(limit.filter(|l| *l > 0)));
    }
}

fn stopped() -> Error {
    Error::Internal("queue is stopped".to_string())
}

struct Running {
    controller: DownloadController,
    spec: DownloadSpec,
    db_row: Option<i64>,
    pause: Arc<AtomicBool>,
}

async fn supervisor(
    mut config: QueueConfig,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    ev_tx: mpsc::Sender<QueueEvent>,
    db: Option<Database>,
) {
    config.max_parallel = config.max_parallel.max(1);
    let limiter = SpeedLimiter::new(config.speed_limit);
    let mut pending: VecDeque<(u64, DownloadSpec, Option<i64>)> = VecDeque::new();
    let mut running: HashMap<u64, Running> = HashMap::new();
    let mut paused: HashMap<u64, (DownloadSpec, Option<i64>)> = HashMap::new();
    let mut next_id = 1u64;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<(u64, bool)>();

    loop {
        while running.len() < config.max_parallel {
            let Some((id, mut spec, db_row)) = pending.pop_front() else {
                break;
            };
            spec.limiter = Some(limiter.clone());
            let target = spec.target.clone();
            let db_ctx = db_row.and_then(|row| db.as_ref().map(|db| (db.clone(), row)));
            let pause = Arc::new(AtomicBool::new(false));
            let download = Download::start(spec.clone());
            let (events, controller, task) = download.into_parts();
            running.insert(
                id,
                Running {
                    controller,
                    spec,
                    db_row,
                    pause: pause.clone(),
                },
            );
            spawn_relay(Relay {
                id,
                target,
                events,
                task,
                ev_tx: ev_tx.clone(),
                done_tx: done_tx.clone(),
                db: db_ctx,
                pause,
            });
        }

        tokio::select! {
            biased;
            cmd = cmd_rx.recv() => match cmd {
                Some(Command::Add { spec, reply }) => {
                    let id = next_id;
                    next_id += 1;
                    // db failures must not block downloads
                    let db_row = db.as_ref().and_then(|db| db.insert(&spec).ok());
                    pending.push_back((id, spec, db_row));
                    let _ = reply.send(id);
                }
                Some(Command::Cancel(id)) => {
                    let before = pending.len();
                    let mut cancelled_row = None;
                    pending.retain(|(pid, _, row)| {
                        if *pid == id {
                            cancelled_row = *row;
                            false
                        } else {
                            true
                        }
                    });
                    if pending.len() != before {
                        if let (Some(db), Some(row)) = (db.as_ref(), cancelled_row) {
                            let _ = db.finish(row, State::Cancelled);
                        }
                        let _ = ev_tx.send(QueueEvent::Cancelled { id }).await;
                    } else if let Some(entry) = running.get(&id) {
                        entry.controller.cancel();
                    } else if let Some((_, row)) = paused.remove(&id) {
                        if let (Some(db), Some(row)) = (db.as_ref(), row) {
                            let _ = db.finish(row, State::Cancelled);
                        }
                        let _ = ev_tx.send(QueueEvent::Cancelled { id }).await;
                    }
                }
                Some(Command::Pause(id)) => {
                    let before = pending.len();
                    let mut taken = None;
                    pending.retain(|(pid, spec, row)| {
                        if *pid == id {
                            taken = Some((spec.clone(), *row));
                            false
                        } else {
                            true
                        }
                    });
                    if pending.len() != before {
                        let (spec, row) = taken.expect("retain captured the paused spec");
                        if let (Some(db), Some(row)) = (db.as_ref(), row) {
                            let _ = db.set_state(row, State::Paused);
                        }
                        paused.insert(id, (spec, row));
                        let _ = ev_tx.send(QueueEvent::Paused { id }).await;
                    } else if let Some(entry) = running.get(&id) {
                        entry.pause.store(true, Ordering::Relaxed);
                        entry.controller.cancel();
                    }
                }
                Some(Command::Resume(id)) => {
                    if let Some((spec, row)) = paused.remove(&id) {
                        if let (Some(db), Some(row)) = (db.as_ref(), row) {
                            let _ = db.set_state(row, State::Queued);
                        }
                        pending.push_back((id, spec, row));
                    }
                }
                Some(Command::SetMaxParallel(max)) => {
                    config.max_parallel = max.max(1);
                }
                Some(Command::SetSpeedLimit(limit)) => {
                    limiter.set_limit(limit);
                }
                None => break,
            },
            Some((id, was_paused)) = done_rx.recv() => {
                if let Some(entry) = running.remove(&id)
                    && was_paused
                {
                    if let (Some(db), Some(row)) = (db.as_ref(), entry.db_row) {
                        let _ = db.set_state(row, State::Paused);
                    }
                    paused.insert(id, (entry.spec, entry.db_row));
                    let _ = ev_tx.send(QueueEvent::Paused { id }).await;
                }
            }
        }
    }
}

struct Relay {
    id: u64,
    target: PathBuf,
    events: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<Result<Outcome, Error>>,
    ev_tx: mpsc::Sender<QueueEvent>,
    done_tx: mpsc::UnboundedSender<(u64, bool)>,
    db: Option<(Database, i64)>,
    pause: Arc<AtomicBool>,
}

fn spawn_relay(relay: Relay) {
    let Relay {
        id,
        target,
        mut events,
        task,
        ev_tx,
        done_tx,
        db,
        pause,
    } = relay;
    tokio::spawn(async move {
        let mut trying = false;
        while let Some(event) = events.recv().await {
            if let Some((db, row)) = db.as_ref() {
                match &event {
                    Event::Started { .. } if !trying => {
                        trying = true;
                        let _ = db.set_state(*row, State::Downloading);
                    }
                    Event::Progress {
                        downloaded, total, ..
                    } => {
                        let _ = db.set_progress(*row, *downloaded, *total);
                    }
                    _ => {}
                }
            }
            let queue_event = match event {
                Event::Started {
                    total,
                    connections,
                    resumed,
                } => QueueEvent::Started {
                    id,
                    path: target.clone(),
                    total,
                    connections,
                    resumed,
                },
                Event::Progress {
                    downloaded,
                    total,
                    speed_bps,
                } => QueueEvent::Progress {
                    id,
                    downloaded,
                    total,
                    speed_bps,
                },
                Event::Finished { path } => QueueEvent::Finished { id, path },
            };
            if ev_tx.send(queue_event).await.is_err() {
                break;
            }
        }
        let outcome = task.await;
        // a pause is a cancel the supervisor asked for; it keeps the .part for resume
        let was_paused =
            pause.load(Ordering::Relaxed) && matches!(&outcome, Ok(Ok(Outcome::Cancelled)));
        let summary = if was_paused {
            None
        } else {
            match outcome {
                Ok(Ok(Outcome::Completed)) => None,
                Ok(Ok(Outcome::Cancelled)) => Some(QueueEvent::Cancelled { id }),
                Ok(Err(e)) => Some(QueueEvent::Failed {
                    id,
                    error: e.to_string(),
                }),
                Err(e) => Some(QueueEvent::Failed {
                    id,
                    error: e.to_string(),
                }),
            }
        };
        if !was_paused && let Some((db, row)) = db.as_ref() {
            match &summary {
                None => {
                    let _ = db.finish(*row, State::Completed);
                }
                Some(QueueEvent::Cancelled { .. }) => {
                    let _ = db.finish(*row, State::Cancelled);
                }
                Some(QueueEvent::Failed { error, .. }) => {
                    let _ = db.set_error(*row, error);
                    let _ = db.finish(*row, State::Failed);
                }
                _ => {}
            }
        }
        if let Some(queue_event) = summary {
            let _ = ev_tx.send(queue_event).await;
        }
        let _ = done_tx.send((id, was_paused));
    });
}
