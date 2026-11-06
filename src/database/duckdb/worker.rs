use std::{
  sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
  },
  thread,
  time::Duration,
};

use color_eyre::eyre::{Result, eyre};
use duckdb::{Config, Connection, InterruptHandle};
use tokio::sync::oneshot::{self, error::TryRecvError};

const INTERRUPT_INTERVAL: Duration = Duration::from_millis(10);

struct WorkerState {
  active: Option<Arc<AtomicBool>>,
  shutdown: bool,
}

struct WorkerControl {
  interrupt: Arc<InterruptHandle>,
  state: Mutex<WorkerState>,
}

pub(super) struct Request<T> {
  response: oneshot::Receiver<Result<T>>,
  cancelled: Arc<AtomicBool>,
  control: Arc<WorkerControl>,
}

impl<T> Request<T> {
  /// The outer error means the worker stopped; the inner result belongs to the request.
  pub fn try_result(&mut self) -> Result<Option<Result<T>>> {
    match self.response.try_recv() {
      Ok(result) => Ok(Some(result)),
      Err(TryRecvError::Empty) => Ok(None),
      Err(TryRecvError::Closed) => Err(eyre!("DuckDB worker stopped before returning a result")),
    }
  }

  pub async fn wait(mut self) -> Result<T> {
    self.receive().await?
  }

  async fn receive(&mut self) -> Result<Result<T>> {
    (&mut self.response).await.map_err(|_| eyre!("DuckDB worker stopped before returning a result"))
  }

  pub fn cancel(&self) {
    self.cancelled.store(true, Ordering::Relaxed);
    let state = self.control.state.lock().unwrap();
    // Hold the state lock through the interrupt so it cannot hit the next request.
    if state.active.as_ref().is_some_and(|active| Arc::ptr_eq(active, &self.cancelled)) {
      self.control.interrupt.interrupt();
    }
  }

  pub async fn cancel_and_wait(&mut self) -> Result<()> {
    loop {
      // DuckDB can reset an interrupt between preparation and execution.
      self.cancel();
      tokio::select! {
        result = self.receive() => {
          let _ = result?;
          return Ok(());
        },
        _ = tokio::time::sleep(INTERRUPT_INTERVAL) => {},
      }
    }
  }
}

impl<T> Drop for Request<T> {
  fn drop(&mut self) {
    self.cancel();
  }
}

type JobFn = Box<dyn FnOnce(&Connection, &WorkerControl) -> bool + Send>;

struct Job {
  // Returns false when shutdown prevents the job from starting.
  run: JobFn,
  span: tracing::Span,
}

pub(super) struct Worker {
  commands: Option<mpsc::Sender<Job>>,
  control: Arc<WorkerControl>,
  thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
  pub async fn open(path: String) -> Result<Self> {
    let (commands, jobs) = mpsc::channel::<Job>();
    let (ready_tx, ready_rx) = oneshot::channel();
    let thread = thread::Builder::new().name("rainfrog-duckdb".into()).spawn(move || {
      let connection = match Connection::open_with_flags(path, Config::default()) {
        Ok(connection) => connection,
        Err(error) => {
          let _ = ready_tx.send(Err(eyre!(error)));
          return;
        },
      };
      let control = Arc::new(WorkerControl {
        interrupt: connection.interrupt_handle(),
        state: Mutex::new(WorkerState { active: None, shutdown: false }),
      });
      if ready_tx.send(Ok(control.clone())).is_err() {
        return;
      }
      for job in jobs {
        if !job.span.in_scope(|| (job.run)(&connection, &control)) {
          break;
        }
      }
    })?;
    let control =
      ready_rx.await.map_err(|_| eyre!("DuckDB worker stopped during initialization"))??;
    Ok(Self { commands: Some(commands), control, thread: Some(thread) })
  }

  pub fn submit<T: Send + 'static>(
    &self,
    operation: impl FnOnce(&Connection, &AtomicBool) -> Result<T> + Send + 'static,
  ) -> Result<Request<T>> {
    let (response_tx, response) = oneshot::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let job_cancelled = cancelled.clone();
    let job = Job {
      span: tracing::Span::current(),
      run: Box::new(move |connection, control| {
        let mut state = control.state.lock().unwrap();
        if state.shutdown {
          return false;
        }
        state.active = Some(job_cancelled.clone());
        drop(state);

        let result =
          check_cancelled(&job_cancelled).and_then(|()| operation(connection, &job_cancelled));
        // Clear the active request before publishing its result or starting another job.
        control.state.lock().unwrap().active = None;
        let _ = response_tx.send(result);
        true
      }),
    };
    self.commands.as_ref().unwrap().send(job).map_err(|_| eyre!("DuckDB worker is unavailable"))?;
    Ok(Request { response, cancelled, control: self.control.clone() })
  }
}

impl Drop for Worker {
  fn drop(&mut self) {
    {
      let mut state = self.control.state.lock().unwrap();
      state.shutdown = true;
      if let Some(active) = &state.active {
        active.store(true, Ordering::Relaxed);
        self.control.interrupt.interrupt();
      }
    }
    drop(self.commands.take());
    if let Some(thread) = self.thread.take() {
      if thread.is_finished() {
        let _ = thread.join();
      } else {
        let control = self.control.clone();
        // Reap off the UI thread, including when the async runtime is shutting down.
        thread::spawn(move || {
          while !thread.is_finished() {
            control.interrupt.interrupt();
            thread::sleep(INTERRUPT_INTERVAL);
          }
          let _ = thread.join();
        });
      }
    }
  }
}

pub(super) fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
  if cancelled.load(Ordering::Relaxed) { Err(eyre!("DuckDB query cancelled")) } else { Ok(()) }
}
