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
use tokio::time::{Instant, sleep_until};

use crate::lifecycle::{TIMEOUT_CODE, hold};
use crate::spawn::Wiring;
use crate::teardown::Tree;
use crate::{Completion, ExecError, ExecRequest, code_of, signals, spawn};

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

/// One step of a sequence: the script it stands for, why it is in the list, and what it
/// runs. The first two are what a run says about a step while the step is happening.
pub struct Step<'a> {
  pub name: String,
  pub role: Role,
  pub task: Task<'a>,
}

/// How a step came to be in the sequence it is in. It decides what is said about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
  /// Named by the group that holds it. Announced as it starts, and named when it fails.
  Member,
  /// Named by the script that has to wait for it. Named only when it fails: `dependsOn`
  /// exists to be invisible when it works.
  Prerequisite,
  /// The declaring script's own command. The user asked for it by name, so its exit code
  /// is already an answer they can attribute.
  Own,
}

/// Everything one `rune run` invocation stands for.
pub enum Task<'a> {
  /// One command, and one child process.
  Command(ExecRequest<'a>),
  /// Each step run to completion, in order.
  Serial { script: String, steps: Vec<Step<'a>>, continue_on_error: bool },
  /// Every member started at once, and all of them waited for.
  Parallel { members: Vec<Member<'a>>, continue_on_error: bool, policy: SuccessPolicy },
}

/// Runs one script, with nothing running beside it.
///
/// The same leaf every group member goes through, so a lone script and a group of one
/// cannot drift apart on spawning, waiting or exit codes.
pub async fn alone(request: &ExecRequest<'_>) -> Result<Completion, ExecError> {
  let run = Run { events: channel::channel().0, color: ColorLevel::None };

  attempts(request, Sink::Inherited, &run, &Control::default()).await
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
    let body = step(task, &labels, Sink::Inherited, &run, &root);

    // A child in rune's own process group already receives whatever the terminal sends,
    // and rune's part is to wait rather than to act. Only a child rune placed out of that
    // group has to be reached, and only then is the guard worth starting.
    if !out_of_reach(task) {
      return body.await;
    }

    return tokio::select! {
      outcome = body => outcome,
      () = guard(&root, None) => unreachable!("the guard waits forever once it has acted"),
    };
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
        () = guard(&root, Some(closed)) => unreachable!("the guard waits forever once it has acted"),
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
/// A run with nothing to write has no output that can close, and passes no receiver.
async fn guard(root: &Control, closed: Option<oneshot::Receiver<()>>) {
  let mut interrupts = signals::interrupts();

  match closed {
    Some(closed) => {
      tokio::select! {
        _ = closed => {}
        _ = interrupts.wait_for(Option::is_some) => {}
      }
    }
    None => {
      let _ = interrupts.wait_for(Option::is_some).await;
    }
  }

  // Each tree gets the wait its own script asked for, so one member's patience is never
  // spent on another's.
  let mut escalation = root.stop();
  while let Some(at) = escalation {
    sleep_until(at).await;
    escalation = root.escalate(Instant::now());
  }

  // Acted once; from here the run ends when its children do.
  pending::<()>().await;
}

/// Whether anything in this task runs where the terminal's own signals do not reach.
///
/// The answer is the same rule spawning uses to place a child in a process group of its
/// own, asked before anything starts.
fn out_of_reach(task: &Task<'_>) -> bool {
  match task {
    Task::Command(request) => request.wants_own_group(),
    Task::Serial { steps, .. } => steps.iter().any(|step| out_of_reach(&step.task)),
    Task::Parallel { members, .. } => members.iter().any(|member| out_of_reach(&member.task)),
  }
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
      children: steps.iter().map(|step| label(&step.task, next, names)).collect(),
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
  running: RefCell<Vec<Running>>,
  nested: RefCell<Vec<Rc<Control>>>,
  stopping: Cell<bool>,
}

/// One tree running under a branch, and how far its teardown has got.
struct Running {
  tree: Tree,
  asked: bool,
  /// When the tree is ended whatever it wanted. Absent while it is running normally, and
  /// absent after it has been asked with a signal nothing can catch.
  deadline: Option<Instant>,
}

impl Control {
  /// A branch of its own, already stopping if this one is.
  fn nest(&self) -> Rc<Self> {
    let nested = Rc::new(Self::default());
    nested.stopping.set(self.stopping.get());
    self.nested.borrow_mut().push(Rc::clone(&nested));

    nested
  }

  /// Drops a branch that has finished, so the book-keeping of a retried script does not
  /// grow with every attempt.
  fn unnest(&self, branch: &Rc<Self>) {
    self.nested.borrow_mut().retain(|nested| !Rc::ptr_eq(nested, branch));
  }

  fn enter(&self, tree: Tree) {
    let mut running = Running { tree, asked: false, deadline: None };

    // A child that started while its branch was being torn down would otherwise be the
    // one process nobody ends.
    if self.stopping.get() {
      ask(&mut running);
    }

    self.running.borrow_mut().push(running);
  }

  fn leave(&self, leader: u32) {
    self.running.borrow_mut().retain(|running| running.tree.leader() != leader);
  }

  /// Asks every tree under this branch to end, stops anything new from starting, and says
  /// when the first of them has been patient long enough.
  fn stop(&self) -> Option<Instant> {
    let mut next = None;
    self.walk(true, &mut |running| {
      ask(running);
      next = sooner(next, running.deadline);
    });

    next
  }

  /// Ends every tree whose wait has run out, and says when the next one's does.
  fn escalate(&self, now: Instant) -> Option<Instant> {
    let mut next = None;
    self.walk(false, &mut |running| {
      let Some(deadline) = running.deadline else {
        return;
      };

      if deadline > now {
        next = sooner(next, Some(deadline));
        return;
      }

      // Asked once and still here: nothing catches this one.
      let _ = running.tree.kill();
      running.deadline = None;
    });

    next
  }

  fn walk(&self, halt: bool, act: &mut dyn FnMut(&mut Running)) {
    if halt {
      self.stopping.set(true);
    }

    for running in self.running.borrow_mut().iter_mut() {
      act(running);
    }
    for nested in self.nested.borrow().iter() {
      nested.walk(halt, act);
    }
  }
}

/// Asks one tree to end with the signal its script chose, and notes when that patience
/// runs out. Asking twice would push the deadline further out every time, so it is asked
/// exactly once.
fn ask(running: &mut Running) {
  if running.asked {
    return;
  }

  running.asked = true;
  let _ = running.tree.terminate();
  running.deadline = running.tree.policy().escalation(Instant::now());
}

/// The earlier of two deadlines, either of which may be absent.
fn sooner(held: Option<Instant>, candidate: Option<Instant>) -> Option<Instant> {
  match (held, candidate) {
    (Some(held), Some(candidate)) => Some(held.min(candidate)),
    (held, candidate) => held.or(candidate),
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
      Task::Command(request) => attempts(request, sink, run, control).await,
      Task::Serial { script, steps, continue_on_error } => {
        serial(script, steps, labels, *continue_on_error, sink, run, control).await
      }
      Task::Parallel { members, continue_on_error, policy } => {
        parallel(members, labels, *continue_on_error, *policy, sink, run, control).await
      }
    }
  })
}

/// Runs each step to completion, in order, and reports how the whole sequence ended.
async fn serial(
  script: &str,
  steps: &[Step<'_>],
  labels: &Labels,
  continue_on_error: bool,
  sink: Sink,
  run: &Run,
  control: &Control,
) -> Result<Completion, ExecError> {
  let mut failure = None;
  let mut caught = None;

  for (index, current) in steps.iter().enumerate() {
    // Nothing new starts once a signal has arrived or a sibling has failed. The flag
    // travels with the previous step's result rather than in a global, so a caller cannot
    // silently skip reading it.
    if caught.is_some() || control.stopping.get() {
      break;
    }

    if current.role == Role::Member {
      rune_out::diagnostic(&starting(script, &current.name));
    }

    let completion = step(&current.task, child(labels, index), sink, run, control).await?;
    caught = caught.or(completion.caught_signal);

    if completion.code != 0 {
      // Said where it happened, so a red log holds the account above the output it
      // explains rather than after everything the run went on to write.
      announce_failure(script, current, index, steps.len(), completion.code);

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

/// A step of a group, as it starts. A step that prints nothing of its own would otherwise
/// leave a log where not running and running silently look the same.
fn starting(script: &str, member: &str) -> String {
  format!("→ {script}: {member}")
}

/// What a failed step is worth saying, which is not the same for the three roles.
///
/// A member is what the group is made of, so the group names it and says where in the
/// sequence it sits. A prerequisite is invisible until it fails, and then the only thing
/// the user has is an exit code from a script they never named. A script's own command
/// needs neither: they asked for it by name.
fn announce_failure(script: &str, step: &Step<'_>, index: usize, total: usize, code: u32) {
  let line = match step.role {
    Role::Member => {
      format!("{script} failed at step {} of {total}: `{}` exited {code}", index + 1, step.name)
    }
    Role::Prerequisite => {
      format!("{script} stopped: prerequisite `{}` exited {code}", step.name)
    }
    Role::Own => return,
  };

  rune_out::diagnostic(&line);
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
          escalation = begin(&controls);
        }
      }
      () = expiry(escalation) => escalation = escalate(&controls, Instant::now()),
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

/// Asks every member's tree to end, and says when the first of them must be ended for it.
fn begin(controls: &[Rc<Control>]) -> Option<Instant> {
  controls.iter().fold(None, |next, branch| sooner(next, branch.stop()))
}

/// Ends every member's tree whose wait has run out, and says when the next one's does.
fn escalate(controls: &[Rc<Control>], now: Instant) -> Option<Instant> {
  controls.iter().fold(None, |next, branch| sooner(next, branch.escalate(now)))
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

/// One command, run again while it fails and attempts remain.
///
/// The loop sits *inside* the unit a group observes, which is the whole of the retry
/// contract: a member that fails once and then succeeds is one success to everything
/// downstream. Outside it, a group's kill-others policy would fire on the first attempt's
/// failure and tear down the siblings of a script that was about to succeed.
async fn attempts(
  request: &ExecRequest<'_>,
  sink: Sink,
  run: &Run,
  control: &Control,
) -> Result<Completion, ExecError> {
  let lifecycle = request.lifecycle;
  let mut attempt = 0;

  loop {
    // A branch of its own per attempt: a timeout ends this attempt's tree without closing
    // the branch the attempts after it still have to start in.
    let branch = control.nest();
    let outcome = once(request, sink, run, &branch, lifecycle.timeout).await;
    control.unnest(&branch);

    let outcome = outcome?;
    let retryable = outcome.completion.code != 0
      && attempt < lifecycle.retries
      // A run being torn down, or one the user interrupted, is over. Starting the next
      // attempt would be rune reviving what something else just ended.
      && outcome.completion.caught_signal.is_none()
      && !control.stopping.get();

    if !retryable {
      // The timeout code is rune's answer, not the child's, and only the attempt nobody
      // follows gets to give it.
      let code = if outcome.timed_out { TIMEOUT_CODE } else { outcome.completion.code };
      return Ok(Completion { code, ..outcome.completion });
    }

    attempt += 1;
    hold(lifecycle.retry_delay, attempt).await;
  }
}

/// How one attempt ended, and whether rune ended it for outliving its budget.
struct Attempt {
  completion: Completion,
  timed_out: bool,
}

/// One attempt, ended through the escalation path if it outlives its timeout.
///
/// The child is never dropped for being late: the future waiting on it keeps being polled
/// while the teardown runs, so the process is still collected and its pipes still drained.
async fn once(
  request: &ExecRequest<'_>,
  sink: Sink,
  run: &Run,
  branch: &Control,
  timeout: Option<Duration>,
) -> Result<Attempt, ExecError> {
  let running = leaf(request, sink, run, branch);

  let Some(timeout) = timeout else {
    return Ok(Attempt { completion: running.await?, timed_out: false });
  };

  tokio::pin!(running);
  let mut deadline = Some(Instant::now() + timeout);
  let mut escalation = None;
  let mut timed_out = false;

  let completion = loop {
    tokio::select! {
      outcome = &mut running => break outcome?,
      () = expiry(deadline) => {
        deadline = None;
        timed_out = true;
        rune_out::diagnostic(&overran(request.script_name, timeout));
        escalation = branch.stop();
      }
      () = expiry(escalation) => escalation = branch.escalate(Instant::now()),
    }
  };

  Ok(Attempt { completion, timed_out })
}

/// Why a script rune ended is reported before its exit code is: 124 on its own is
/// indistinguishable from a child that chose to exit 124.
fn overran(script: &str, timeout: Duration) -> String {
  format!(
    "`{script}` exceeded its {} ms timeout — its process tree was terminated",
    timeout.as_millis()
  )
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
