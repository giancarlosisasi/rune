//! What the child sees of the terminal.
//!
//! Inherited standard streams are the product's differentiator, so these run against a
//! real pseudoterminal rather than being skipped. A "done when" that rests on an ignored
//! test is not a "done when".

mod harness;

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(unix)]
use harness::READY;
use harness::{Test, binary, pinned_shell, wait_until};
use portable_pty::{CommandBuilder, MasterPty, PtySize, SlavePty, native_pty_system};

/// Bounds a failure. Nothing here is sequenced by it — the fixture announces itself.
const RESPONSE_LIMIT: Duration = Duration::from_secs(20);

/// A terminal is expected to report where its cursor is when asked. Windows asks as part
/// of starting a pseudoconsole and holds the child until an answer arrives, so a harness
/// that stays silent never sees the child run at all.
const CURSOR_QUERY: &str = "\u{1b}[6n";
const CURSOR_REPORT: &[u8] = b"\x1b[1;1R";

const TOOL: &str = "faketool.exe";

/// The writing end of the pseudoterminal, shared with the thread that answers the
/// terminal queries the child makes.
type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

fn config(command: &str) -> String {
  format!("export default {{ scripts: {{ probe: {{ command: \"{command}\" }} }} }};\n")
}

/// The same command, reached through a serial group rather than run by name.
fn chained_config(command: &str) -> String {
  format!(
    "export default {{ scripts: {{ \
     step: {{ command: \"{command}\" }}, \
     probe: {{ serial: [\"step\"] }} \
     }} }};\n"
  )
}

/// Test 3.17 — colors, progress bars and watch-mode interfaces all depend on this one
/// answer being true.
#[test]
fn under_a_terminal_the_child_sees_a_terminal_on_all_three_descriptors() {
  let test = Test::new()
    .config(&config(&format!("{TOOL} isatty")))
    .tool(&format!("node_modules/.bin/{TOOL}"));

  let mut session = Session::start(&test);
  session.await_text("\"stdin\"");
  let status = session.wait();
  let transcript = session.transcript();

  assert!(status.success(), "rune exited {status:?}\n{transcript}");

  let report = report_in(&transcript);
  for stream in ["stdin", "stdout", "stderr"] {
    assert_eq!(report[stream], true, "{stream} was not a terminal\n{transcript}");
  }
}

/// Test 5a.9's secondary check — a member of a chain owns the terminal exactly as a lone
/// script does.
///
/// The byte-equality assertion in `composition.rs` is the cheap primary one. This is what
/// says the promise is inheritance rather than a faithful copy: the terminal itself is
/// what the child is given.
#[test]
fn a_child_inside_a_serial_chain_still_sees_a_terminal() {
  let test = Test::new()
    .config(&chained_config(&format!("{TOOL} isatty")))
    .tool(&format!("node_modules/.bin/{TOOL}"));

  let mut session = Session::start(&test);
  session.await_text("\"stdin\"");
  let status = session.wait();
  let transcript = session.transcript();

  assert!(status.success(), "rune exited {status:?}\n{transcript}");

  let report = report_in(&transcript);
  for stream in ["stdin", "stdout", "stderr"] {
    assert_eq!(report[stream], true, "{stream} was not a terminal in a chain\n{transcript}");
  }
}

/// Test 5c.10, first half — an interactive member keeps the terminal while its sibling
/// stays piped and prefixed.
///
/// This is the exemption stated in design D5, and the group is where it earns its keep: a
/// watch interface inside a `dev` group is the case the whole feature exists to serve.
#[test]
fn an_interactive_member_keeps_the_terminal_while_its_sibling_stays_prefixed() {
  let test = Test::new()
    .config(&format!(
      "export default {{ scripts: {{ \
       watch: {{ command: \"{TOOL} isatty\", interactive: true }}, \
       noisy: {{ command: \"{TOOL} chatty SIBLING 2\" }}, \
       probe: {{ parallel: [\"watch\", \"noisy\"] }} \
       }} }};\n"
    ))
    .tool(&format!("node_modules/.bin/{TOOL}"));

  let mut session = Session::start(&test);
  // Both members end on their own, so the run is waited out rather than watched. Polling
  // a transcript for one member's line while the other is still writing asserts on a race
  // rather than on the behavior.
  let status = session.wait();
  session.await_text("[noisy] SIBLING 2");
  let transcript = session.transcript();

  assert!(status.success(), "rune exited {status:?}\n{transcript}");

  let report = report_in(&transcript);
  for stream in ["stdin", "stdout", "stderr"] {
    assert_eq!(report[stream], true, "{stream} was not a terminal\n{transcript}");
  }

  assert!(
    !transcript.contains("[watch]"),
    "the interactive member's output went through the writer\n{transcript}"
  );
}

/// Test 5c.10, second half, and the reason the exemption exists at all.
///
/// A process outside the terminal's foreground group is stopped by `SIGTTIN` the moment it
/// reads the terminal. Piped members are placed in groups of their own so their trees can
/// be torn down, so an interactive member has to be left out of that — or every watch mode
/// in a group freezes. The child answering at all is the assertion.
#[test]
#[cfg(unix)]
fn an_interactive_member_reading_the_terminal_keeps_running() {
  let test = Test::new()
    .config(&format!(
      "export default {{ scripts: {{ \
       watch: {{ command: \"exec {TOOL} ready-then-wait\", interactive: true }}, \
       noisy: {{ command: \"{TOOL} chatty SIBLING 2\" }}, \
       probe: {{ parallel: [\"watch\", \"noisy\"] }} \
       }} }};\n"
    ))
    .tool(&format!("node_modules/.bin/{TOOL}"));

  let mut session = Session::start(&test);
  session.await_text(READY);
  session.send("hello\n");
  session.await_text("GOT hello");

  let status = session.wait();
  assert!(status.success(), "rune exited {status:?}\n{}", session.transcript());
}

/// Test 3.18 — the row that would have caught the original design.
///
/// A process outside the terminal's foreground group is stopped by `SIGTTIN` the moment
/// it reads the terminal. The child answering at all is the assertion.
#[test]
#[cfg(unix)]
fn a_child_reading_the_terminal_keeps_running() {
  let test = Test::new()
    .config(&config(&format!("exec {TOOL} ready-then-wait")))
    .tool(&format!("node_modules/.bin/{TOOL}"));

  let mut session = Session::start(&test);
  session.await_text(READY);
  session.send("hello\n");
  session.await_text("GOT hello");

  let status = session.wait();
  assert!(status.success(), "rune exited {status:?}\n{}", session.transcript());
}

/// A run of `rune` attached to a real pseudoterminal.
struct Session {
  child: Box<dyn portable_pty::Child + Send + Sync>,
  /// Held for the whole session. Dropping either end closes the pseudoterminal, and on
  /// Windows the slave owns the pseudoconsole itself — closing it takes the child's
  /// output with it. Unix wants the opposite: the slave goes, so reads reach end of file.
  _master: Box<dyn MasterPty + Send>,
  _slave: Option<Box<dyn SlavePty + Send>>,
  #[cfg_attr(windows, expect(dead_code, reason = "held open: closing it interrupts the child"))]
  writer: Writer,
  transcript: Arc<Mutex<String>>,
}

impl Session {
  fn start(test: &Test) -> Self {
    let pty = native_pty_system()
      .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
      .expect("open a pseudoterminal");

    let mut command = CommandBuilder::new(binary());
    command.args(["run", "probe"]);
    command.cwd(test.dir());
    command.env("npm_config_script_shell", pinned_shell());
    command.env("FORCE_COLOR", "0");
    command.env_remove("NO_COLOR");
    command.env_remove("CI");

    // Taken before the child exists and kept until the session ends: closing the writing
    // end of a pseudoconsole reads to the child as an interrupt, and it dies instead of
    // running.
    let writer: Writer =
      Arc::new(Mutex::new(pty.master.take_writer().expect("write to the pseudoterminal")));
    let reader = pty.master.try_clone_reader().expect("read the pseudoterminal");

    let child = pty.slave.spawn_command(command).expect("spawn rune under the pseudoterminal");

    let transcript = Arc::new(Mutex::new(String::new()));
    collect(reader, Arc::clone(&transcript), Arc::clone(&writer));

    Self {
      child,
      _master: pty.master,
      _slave: cfg!(windows).then_some(pty.slave),
      writer,
      transcript,
    }
  }

  fn await_text(&self, wanted: &str) {
    assert!(
      wait_until(RESPONSE_LIMIT, || self.transcript().contains(wanted)),
      "never saw {wanted:?}\n{}",
      self.transcript()
    );
  }

  #[cfg(unix)]
  fn send(&self, text: &str) {
    let mut writer = self.writer.lock().expect("the pseudoterminal writer lock");
    writer.write_all(text.as_bytes()).expect("write to the pseudoterminal");
    writer.flush().expect("flush the pseudoterminal");
  }

  fn wait(&mut self) -> portable_pty::ExitStatus {
    self.child.wait().expect("collect rune")
  }

  fn transcript(&self) -> String {
    self.transcript.lock().expect("the transcript lock").clone()
  }
}

impl Drop for Session {
  fn drop(&mut self) {
    // A failed assertion must not leave rune holding the pseudoterminal: the test process
    // would then wait out its own timeout for a child that is never coming back. An
    // already-exited child is the ordinary case here, not an error.
    let _ = self.child.kill();
  }
}

/// Reads the terminal into `transcript`, answering the queries a child makes of it.
fn collect(mut reader: Box<dyn Read + Send>, transcript: Arc<Mutex<String>>, writer: Writer) {
  std::thread::spawn(move || {
    let mut buffer = [0_u8; 1024];
    while let Ok(read) = reader.read(&mut buffer) {
      if read == 0 {
        return;
      }

      let chunk = String::from_utf8_lossy(&buffer[..read]);
      if chunk.contains(CURSOR_QUERY) {
        let mut writer = writer.lock().expect("the pseudoterminal writer lock");
        let _ = writer.write_all(CURSOR_REPORT).and_then(|()| writer.flush());
      }

      transcript.lock().expect("the transcript lock").push_str(&chunk);
    }
  });
}

/// The fixture's JSON report, dug out of a transcript that also carries the terminal's own
/// escape sequences — which arrive on the same line as the report, so splitting on
/// newlines does not find it.
fn report_in(transcript: &str) -> serde_json::Value {
  let start =
    transcript.find('{').unwrap_or_else(|| panic!("no report in the transcript:\n{transcript}"));

  serde_json::Deserializer::from_str(&transcript[start..])
    .into_iter::<serde_json::Value>()
    .next()
    .and_then(Result::ok)
    .unwrap_or_else(|| panic!("no report in the transcript:\n{transcript}"))
}
