use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

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
}

enum Command {
    Add {
        spec: DownloadSpec,
        reply: oneshot::Sender<u64>,
    },
    Cancel(u64),
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

async fn supervisor(
    mut config: QueueConfig,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    ev_tx: mpsc::Sender<QueueEvent>,
    db: Option<Database>,
) {
    config.max_parallel = config.max_parallel.max(1);
    let limiter = SpeedLimiter::new(config.speed_limit);
    let mut pending: VecDeque<(u64, DownloadSpec, Option<i64>)> = VecDeque::new();
    let mut running: HashMap<u64, DownloadController> = HashMap::new();
    let mut next_id = 1u64;
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<u64>();

    loop {
        while running.len() < config.max_parallel {
            let Some((id, mut spec, db_row)) = pending.pop_front() else {
                break;
            };
            spec.limiter = Some(limiter.clone());
            let download = Download::start(spec);
            let (events, controller, task) = download.into_parts();
            running.insert(id, controller);
            spawn_relay(
                id,
                events,
                task,
                ev_tx.clone(),
                done_tx.clone(),
                db.clone(),
                db_row,
            );
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
                    } else if let Some(controller) = running.get(&id) {
                        controller.cancel();
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
            Some(id) = done_rx.recv() => {
                running.remove(&id);
            }
        }
    }
}

fn spawn_relay(
    id: u64,
    mut events: mpsc::Receiver<Event>,
    task: tokio::task::JoinHandle<Result<Outcome, Error>>,
    ev_tx: mpsc::Sender<QueueEvent>,
    done_tx: mpsc::UnboundedSender<u64>,
    db: Option<Database>,
    db_row: Option<i64>,
) {
    tokio::spawn(async move {
        let mut trying = false;
        while let Some(event) = events.recv().await {
            if let (Some(db), Some(row)) = (db.as_ref(), db_row) {
                match &event {
                    Event::Started { .. } if !trying => {
                        trying = true;
                        let _ = db.set_state(row, State::Downloading);
                    }
                    Event::Progress {
                        downloaded, total, ..
                    } => {
                        let _ = db.set_progress(row, *downloaded, *total);
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
        let summary = match task.await {
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
        };
        if let (Some(db), Some(row)) = (db.as_ref(), db_row) {
            match &summary {
                None => {
                    let _ = db.finish(row, State::Completed);
                }
                Some(QueueEvent::Cancelled { .. }) => {
                    let _ = db.finish(row, State::Cancelled);
                }
                Some(QueueEvent::Failed { error, .. }) => {
                    let _ = db.set_error(row, error);
                    let _ = db.finish(row, State::Failed);
                }
                _ => {}
            }
        }
        if let Some(queue_event) = summary {
            let _ = ev_tx.send(queue_event).await;
        }
        let _ = done_tx.send(id);
    });
}
