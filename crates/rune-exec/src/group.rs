//! Running several scripts at once, and ending them together.
//!
//! Three things make this harder than starting two processes. Output from two children
//! sharing one terminal is unattributable unless something labels it, so every member but
//! an interactive one is piped and its bytes are prefixed on the way through. A member
//! that fails has to take its siblings' *whole trees* with it, because the process rune
//! holds is a shell and the tool doing the work is its child. And every way out of a run —
//! a failure, an interrupt, a closed output, an ordinary finish — has to leave no process
//! behind and still report a code that means something.
//!
//! Two rules keep the answers honest. Exit order is chronological, never declaration
//! order: where a member sits in a list says nothing about when it finishes. And a process
//! that has already exited is never an error to kill, checked on every path.

use std::cell::{Cell, RefCell};
use std::future::pending;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use futures_util::stream::FuturesUnordered;
use rune_out::channel::{self, Event};
use rune_out::color::{ColorLevel, Palette};
use rune_out::multiplex::{Multiplexer, ScriptId, Stream};
use tokio::io::AsyncReadExt as _;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep, sleep_until};

use crate::spawn::Wiring;
use crate::teardown::Tree;
use crate::{Completion, ExecError, ExecRequest, code_of, signals, spawn};

/// How long a tree is given to end on its own before it is ended for it.
///
/// Long enough that a server flushing its state on the way out is not cut off, short
/// enough that a member ignoring the request cannot hold the whole run open. Nothing here
/// sequences anything: on the ordinary path the timer is cancelled long before it fires.
const KILL_TIMEOUT: Duration = Duration::from_secs(5);

/// How much of a pipe is read at once.
const CHUNK: usize = 8 * 1024;

/// Which member's result the group takes as its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SuccessPolicy {
  /// Every member must have succeeded.
  #[default]
  All,
  /// The member that exited first in time.
  First,
  /// The member that exited last in time.
  Last,
}

/// One member of a parallel group: the name its output is labelled with, and what it runs.
pub struct Member<'a> {
  pub name: String,
  pub task: Task<'a>,
}

/// Everything one `rune run` invocation stands for.
pub enum Task<'a> {
  /// One command, and one child process.
  Command(ExecRequest<'a>),
  /// Each step run to completion, in order.
  Serial { steps: Vec<Task<'a>>, continue_on_error: bool },
  /// Every member started at once, and all of them waited for.
  Parallel { members: Vec<Member<'a>>, continue_on_error: bool, policy: SuccessPolicy },
}

/// Runs one script, with nothing running beside it.
///
/// The same leaf every group member goes through, so a lone script and a group of one
/// cannot drift apart on spawning, waiting or exit codes.
pub async fn alone(request: &ExecRequest<'_>) -> Result<Completion, ExecError> {
  let run = Run { events: channel::channel().0, color: ColorLevel::None };

  leaf(request, Sink::Inherited, &run, &Control::default()).await
}

/// Runs everything `task` stands for.
pub async fn execute(task: &Task<'_>) -> Result<Completion, ExecError> {
  let mut names = Vec::new();
  let mut next = 0;
  let labels = label(task, &mut next, &mut names);
  let root = Control::default();

  // Nothing runs beside anything else, so nothing needs labelling and the children keep
  // rune's own terminal exactly as a single script does. No writer either: it would hold
  // rune's own stdout for a run that has nothing to put through it.
  if names.is_empty() {
    let run = Run { events: channel::channel().0, color: ColorLevel::None };
    return step(task, &labels, Sink::Inherited, &run, &root).await;
  }

  let color = ColorLevel::detect();
  let (events, receiver) = channel::channel();
  let (broken, closed) = oneshot::channel();

  let body = async {
    // The sender lives exactly as long as the run: dropping it here is what tells the
    // writer that everything which could produce output has finished.
    let run = Run { events, color };
    step(task, &labels, Sink::Inherited, &run, &root).await
  };

  let (outcome, written) = tokio::join!(
    async {
      tokio::select! {
        outcome = body => outcome,
        () = guard(&root, closed) => unreachable!("the guard waits forever once it has acted"),
      }
    },
    write(receiver, &names, color, broken),
  );

  // A closed output is how the run ended, not a fault to report on the way out: the
  // reader chose to stop reading. The children were torn down for it, and that is enough.
  let _ = written;

  outcome
}

/// Ends the whole run when something outside it says to.
///
/// Both reasons are the same event as far as the children are concerned — the run is over
/// and nothing may survive it — so they share one path rather than two that could drift.
async fn guard(root: &Control, closed: oneshot::Receiver<()>) {
  let mut interrupts = signals::interrupts();

  tokio::select! {
    _ = closed => {}
    _ = interrupts.wait_for(Option::is_some) => {}
  }

  root.stop();
  sleep(KILL_TIMEOUT).await;
  root.kill();

  // Acted once; from here the run ends when its children do.
  pending::<()>().await;
}

/// What every task in one invocation shares.
struct Run {
  events: mpsc::Sender<Event>,
  color: ColorLevel,
}

/// Where one task's bytes go.
#[derive(Debug, Clone, Copy)]
enum Sink {
  /// Rune's own terminal, untouched.
  Inherited,
  /// The writer, labelled as this script.
  Piped(ScriptId),
}

/// The prefix ids one task's parallel groups use, mirroring the task's own shape.
///
/// Assigned before anything runs, so that two groups starting at the same moment cannot
/// race for a number. Reading them back at run time is an index, never a counter.
#[derive(Default)]
struct Labels {
  /// One id per member, when this task is a parallel group that needs prefixes.
  members: Vec<ScriptId>,
  /// One entry per child task, in the order the task holds them.
  children: Vec<Labels>,
}

/// Hands out an id to every member whose output will need a prefix, and collects the
/// names those prefixes are drawn from.
fn label(task: &Task<'_>, next: &mut ScriptId, names: &mut Vec<String>) -> Labels {
  match task {
    Task::Command(_) => Labels::default(),
    Task::Serial { steps, .. } => Labels {
      members: Vec::new(),
      children: steps.iter().map(|step| label(step, next, names)).collect(),
    },
    Task::Parallel { members, .. } => {
      // A group of one has nothing to disambiguate, so it takes no ids and its member
      // keeps the terminal exactly as a single script would.
      let ids = if members.len() < 2 {
        Vec::new()
      } else {
        members
          .iter()
          .map(|member| {
            names.push(member.name.clone());
            let id = *next;
            *next += 1;
            id
          })
          .collect()
      };

      Labels {
        members: ids,
        children: members.iter().map(|member| label(&member.task, next, names)).collect(),
      }
    }
  }
}

/// The lever a teardown pulls on one branch of a run.
///
/// A branch owns the children running under it right now and the branches nested inside
/// it, so ending one member of an outer group ends everything that member started,
/// however deep. `stopping` is what a serial branch reads before starting its next step:
/// after a teardown begins, nothing new may spawn.
#[derive(Default)]
struct Control {
  running: RefCell<Vec<Tree>>,
  nested: RefCell<Vec<Rc<Control>>>,
  stopping: Cell<bool>,
}

impl Control {
  /// A branch of its own, already stopping if this one is.
  fn nest(&self) -> Rc<Self> {
    let nested = Rc::new(Self::default());
    nested.stopping.set(self.stopping.get());
    self.nested.borrow_mut().push(Rc::clone(&nested));

    nested
  }

  fn enter(&self, tree: Tree) {
    // A child that started while its branch was being torn down would otherwise be the
    // one process nobody ends.
    if self.stopping.get() {
      let _ = tree.terminate();
    }

    self.running.borrow_mut().push(tree);
  }

  fn leave(&self, leader: u32) {
    self.running.borrow_mut().retain(|tree| tree.leader() != leader);
  }

  /// Asks every tree under this branch to end, and stops anything new from starting.
  fn stop(&self) {
    self.stopping.set(true);
    self.each(&|tree| {
      let _ = tree.terminate();
    });
  }

  /// Ends every tree under this branch whatever it wanted.
  fn kill(&self) {
    self.each(&|tree| {
      let _ = tree.kill();
    });
  }

  fn each(&self, act: &dyn Fn(&Tree)) {
    for tree in self.running.borrow().iter() {
      act(tree);
    }
    for nested in self.nested.borrow().iter() {
      nested.stopping.set(true);
      nested.each(act);
    }
  }
}

/// Runs one task, whatever kind it is.
///
/// Boxed because the recursion is a value: a task holds tasks, and an `async fn` calling
/// itself needs its future to have a size the compiler can name.
fn step<'f>(
  task: &'f Task<'_>,
  labels: &'f Labels,
  sink: Sink,
  run: &'f Run,
  control: &'f Control,
) -> LocalBoxFuture<'f, Result<Completion, ExecError>> {
  Box::pin(async move {
    match task {
      Task::Command(request) => leaf(request, sink, run, control).await,
      Task::Serial { steps, continue_on_error } => {
        serial(steps, labels, *continue_on_error, sink, run, control).await
      }
      Task::Parallel { members, continue_on_error, policy } => {
        parallel(members, labels, *continue_on_error, *policy, sink, run, control).await
      }
    }
  })
}

/// Runs each step to completion, in order, and reports how the whole sequence ended.
async fn serial(
  steps: &[Task<'_>],
  labels: &Labels,
  continue_on_error: bool,
  sink: Sink,
  run: &Run,
  control: &Control,
) -> Result<Completion, ExecError> {
  let mut failure = None;
  let mut caught = None;

  for (index, task) in steps.iter().enumerate() {
    // Nothing new starts once a signal has arrived or a sibling has failed. The flag
    // travels with the previous step's result rather than in a global, so a caller cannot
    // silently skip reading it.
    if caught.is_some() || control.stopping.get() {
      break;
    }

    let completion = step(task, child(labels, index), sink, run, control).await?;
    caught = caught.or(completion.caught_signal);

    if completion.code != 0 {
      // The first failure's code, never the last. In a serial run first-in-time is also
      // first-in-list, so it is the failure the user reads at the top of the log.
      failure.get_or_insert(completion.code);

      if !continue_on_error {
        break;
      }
    }
  }

  Ok(Completion { code: failure.unwrap_or(0), caught_signal: caught })
}

/// Starts every member at once, waits for all of them, and answers for the group.
async fn parallel(
  members: &[Member<'_>],
  labels: &Labels,
  continue_on_error: bool,
  policy: SuccessPolicy,
  sink: Sink,
  run: &Run,
  control: &Control,
) -> Result<Completion, ExecError> {
  // A group of one is a single script wearing a group's name. Prefixing it would degrade
  // the terminal behavior of a config that happens to wrap one script.
  if let [only] = members {
    return step(&only.task, child(labels, 0), sink, run, control).await;
  }

  let controls: Vec<Rc<Control>> = members.iter().map(|_| control.nest()).collect();

  let mut running = FuturesUnordered::new();
  for (index, member) in members.iter().enumerate() {
    let branch = Rc::clone(&controls[index]);
    let script = labels.members[index];
    let announce = run.events.clone();

    running.push(async move {
      let outcome =
        step(&member.task, child(labels, index), Sink::Piped(script), run, &branch).await;
      // Grouped output holds a script's block until it says it is done. Interleaved
      // output has nothing to do here, and both go through the same seam.
      let _ = announce.send(Event::Finished { script }).await;

      outcome
    });
  }

  let mut exits = Vec::with_capacity(members.len());
  let mut failure = None;
  let mut escalation = None;
  let mut stopping = false;

  while !running.is_empty() {
    tokio::select! {
      finished = running.next() => {
        let Some(outcome) = finished else { break };

        let failed = match outcome {
          Ok(completion) => {
            exits.push(completion);
            completion.code != 0 && !continue_on_error
          }
          Err(error) => {
            // A member that could not start leaves the rest with nothing to wait for.
            failure.get_or_insert(error);
            true
          }
        };

        if failed && !stopping {
          stopping = true;
          escalation = Some(begin(&controls));
        }
      }
      () = expiry(escalation) => {
        escalation = None;
        // Asked once and still here: nothing catches this one.
        for branch in &controls {
          branch.kill();
        }
      }
    }
  }

  if let Some(error) = failure {
    return Err(error);
  }

  Ok(judge(&exits, policy))
}

/// The group's own result, read off the members' exits in the order they happened.
fn judge(exits: &[Completion], policy: SuccessPolicy) -> Completion {
  let caught = exits.iter().find_map(|exit| exit.caught_signal);

  let code = match policy {
    // The first failure in time, which is the one that explains the rest.
    SuccessPolicy::All => exits.iter().find(|exit| exit.code != 0).map_or(0, |exit| exit.code),
    SuccessPolicy::First => exits.first().map_or(0, |exit| exit.code),
    SuccessPolicy::Last => exits.last().map_or(0, |exit| exit.code),
  };

  Completion { code, caught_signal: caught }
}

/// Asks every member's tree to end, and says when patience runs out.
fn begin(controls: &[Rc<Control>]) -> Instant {
  for branch in controls {
    branch.stop();
  }

  Instant::now() + KILL_TIMEOUT
}

/// The escalation deadline, or nothing at all while none is set.
///
/// An absolute instant rather than a live timer, so the caller can rebuild this future on
/// every pass of its loop without ever extending the deadline it is waiting for.
async fn expiry(at: Option<Instant>) {
  match at {
    Some(at) => sleep_until(at).await,
    None => pending().await,
  }
}

/// The labels belonging to one child of a task.
///
/// A task whose shape produced no labels — every plain command — reads back an empty set
/// rather than an absence, so the walk never has to test for one.
fn child(labels: &Labels, index: usize) -> &Labels {
  static NOTHING: Labels = Labels { members: Vec::new(), children: Vec::new() };

  labels.children.get(index).unwrap_or(&NOTHING)
}

/// One child process, started and waited for.
async fn leaf(
  request: &ExecRequest<'_>,
  sink: Sink,
  run: &Run,
  control: &Control,
) -> Result<Completion, ExecError> {
  // An interactive member keeps the terminal wherever it sits. That is the whole of the
  // exemption: a process outside the terminal's foreground group is stopped the moment it
  // reads the terminal, which freezes exactly the watch interfaces this exists for.
  let piped = match sink {
    Sink::Piped(script) if !request.interactive => Some(script),
    _ => None,
  };

  let wiring = piped.map_or(Wiring::Inherited, |_| Wiring::Piped(run.color));
  let mut spawned = spawn::start(request, wiring)?;
  control.enter(spawned.tree.clone());

  let status = match piped {
    None => spawned.child.wait().await,
    Some(script) => {
      let out = spawned.child.stdout.take();
      let err = spawned.child.stderr.take();

      // The streams are drained to their end, not merely until the child exits. A member
      // terminated because a sibling failed has usually already written the line that
      // explains why, and truncating it loses the only evidence there was.
      let ((), (), status) = tokio::join!(
        pipe(out, script, Stream::Stdout, &run.events),
        pipe(err, script, Stream::Stderr, &run.events),
        spawned.child.wait(),
      );

      status
    }
  };

  control.leave(spawned.pid);
  signals::forget(spawned.pid);

  let status =
    status.map_err(|source| ExecError::Wait { script: request.script_name.to_owned(), source })?;

  Ok(Completion { code: code_of(status), caught_signal: signals::caught() })
}

/// Reads one stream to its end, handing every chunk to the writer as it arrives.
async fn pipe<R>(source: Option<R>, script: ScriptId, stream: Stream, events: &mpsc::Sender<Event>)
where
  R: tokio::io::AsyncRead + Unpin,
{
  let Some(mut source) = source else {
    return;
  };

  let mut buffer = vec![0_u8; CHUNK];
  loop {
    let Ok(read) = source.read(&mut buffer).await else {
      return;
    };

    if read == 0 {
      return;
    }

    let chunk = Event::Chunk { script, stream, bytes: buffer[..read].to_vec() };
    if events.send(chunk).await.is_err() {
      // The writer is gone. Reading on would be work with nowhere to put it, and the
      // teardown that follows a closed output is already under way.
      return;
    }
  }
}

/// Writes every chunk to rune's own output, labelled with the script that produced it.
async fn write(
  mut receiver: mpsc::Receiver<Event>,
  names: &[String],
  color: ColorLevel,
  broken: oneshot::Sender<()>,
) -> io::Result<()> {
  let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
  let mut writer = Multiplexer::interleaved(io::stdout().lock(), Palette::new(&borrowed, color));

  let result = channel::pump(&mut receiver, &mut writer).await;
  if result.is_err() {
    // Nothing more will be written, so a member parked on a full queue has to be let go
    // rather than left waiting for a reader that has stopped reading.
    receiver.close();
    while receiver.recv().await.is_some() {}
    let _ = broken.send(());
  }

  result
}
