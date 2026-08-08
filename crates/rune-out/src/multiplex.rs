//! Making the output of several scripts attributable on one terminal.
//!
//! The whole component is a function of a sequence of `(script, stream, chunk)` triples:
//! the same triples always produce the same bytes. Nothing here spawns anything, waits
//! for anything, or reads a clock.
//!
//! It works in **bytes, never in text**. That is not an optimization — it is the reason a
//! multi-byte character split across two reads survives. The reference implementation
//! turns each chunk into a string as it arrives, so half a character becomes a
//! replacement character the child can neither detect nor prevent. Here the only bytes
//! ever inspected are `\n` and `\r`, and in UTF-8 no byte of a multi-byte character can
//! be either of them, so the split never has to be noticed to be handled correctly.

use std::io::{self, Write};

use crate::color::Palette;

/// Which script a chunk came from: its position in the run.
pub type ScriptId = usize;

/// Which of a script's two output streams a chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
  Stdout,
  Stderr,
}

/// One script's one stream. Both halves matter: a script's own two streams must not be
/// spliced onto one line either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Source {
  script: ScriptId,
  stream: Stream,
}

/// Where a run's output goes.
pub struct Multiplexer<W: Write> {
  out: W,
  palette: Palette,
  mode: Mode,
}

enum Mode {
  /// Written as it arrives, character by character.
  ///
  /// The alternative — hold a line until its newline — reads better and makes an
  /// interactive child look hung: `Continue? (y/n)` carries no newline, so the user is
  /// asked to answer a question they cannot see. Fixing that needs a quiescence timer,
  /// which is a timing heuristic in the middle of an otherwise exact component.
  Interleaved(Renderer),
  /// Held per script and written as one block when that script finishes.
  Grouped(Vec<Block>),
}

/// One script's output while it is being held back.
struct Block {
  bytes: Vec<u8>,
  renderer: Renderer,
}

impl<W: Write> Multiplexer<W> {
  pub fn interleaved(out: W, palette: Palette) -> Self {
    Self { out, palette, mode: Mode::Interleaved(Renderer::new()) }
  }

  pub fn grouped(out: W, palette: Palette) -> Self {
    let blocks =
      (0..palette.len()).map(|_| Block { bytes: Vec::new(), renderer: Renderer::new() }).collect();

    Self { out, palette, mode: Mode::Grouped(blocks) }
  }

  pub fn write(&mut self, script: ScriptId, stream: Stream, bytes: &[u8]) -> io::Result<()> {
    let source = Source { script, stream };
    let prefix = self.palette.prefix(script);

    match &mut self.mode {
      Mode::Interleaved(renderer) => renderer.write(&mut self.out, prefix, source, bytes),
      Mode::Grouped(blocks) => {
        let block = &mut blocks[script];
        block.renderer.write(&mut block.bytes, prefix, source, bytes)
      }
    }
  }

  /// Reports that a script produced everything it is going to.
  ///
  /// In grouped mode this is what puts its block on the terminal, which is why the blocks
  /// come out in the order the scripts finished rather than the order they were declared.
  /// A block that appeared before its script had finished would have to be written a
  /// piece at a time, and that is the other mode.
  pub fn finished(&mut self, script: ScriptId) -> io::Result<()> {
    let Mode::Grouped(blocks) = &mut self.mode else {
      return Ok(());
    };

    let block = std::mem::take(&mut blocks[script].bytes);
    self.out.write_all(&block)
  }

  /// Writes out whatever is still held.
  ///
  /// A run can end with a script that never reported finishing — it was killed, or the
  /// run was interrupted. What it managed to say still belongs to the user.
  pub fn finish_all(&mut self) -> io::Result<()> {
    let Mode::Grouped(blocks) = &mut self.mode else {
      return Ok(());
    };

    for block in blocks {
      let held = std::mem::take(&mut block.bytes);
      self.out.write_all(&held)?;
    }

    Ok(())
  }

  /// Pushes whatever is buffered out to where it is going.
  ///
  /// A terminal's own buffering holds a line until it ends, and a child asking a question
  /// or repainting a progress bar has not ended one. Waiting for a newline that is not
  /// coming makes a running script look hung.
  pub fn flush(&mut self) -> io::Result<()> {
    self.out.flush()
  }

  pub fn into_inner(self) -> W {
    self.out
  }
}

/// The prefixing rule, as a state machine over bytes.
struct Renderer {
  /// Which source wrote last, so a switch can be noticed.
  last: Option<Source>,
  /// The last byte written ended a line, so another script may start without a newline
  /// being injected first.
  terminated: bool,
  /// The next byte written needs a prefix in front of it.
  ///
  /// Kept apart from `terminated` because a carriage return needs a prefix again without
  /// ending the line: the child returned to column zero and painted over the old one.
  needs_prefix: bool,
}

impl Renderer {
  fn new() -> Self {
    Self { last: None, terminated: true, needs_prefix: true }
  }

  fn write(
    &mut self,
    out: &mut impl Write,
    prefix: &str,
    source: Source,
    bytes: &[u8],
  ) -> io::Result<()> {
    if bytes.is_empty() {
      return Ok(());
    }

    // Two scripts must never share a line. The newline goes in before the new prefix, so
    // what the first one wrote stays readable and stays attributed to it.
    if self.last != Some(source) && !self.terminated {
      out.write_all(b"\n")?;
      self.terminated = true;
      self.needs_prefix = true;
    }
    self.last = Some(source);

    let mut index = 0;
    while index < bytes.len() {
      let start = index;
      while index < bytes.len() && bytes[index] != b'\n' && bytes[index] != b'\r' {
        index += 1;
      }

      if index > start {
        self.open(out, prefix)?;
        out.write_all(&bytes[start..index])?;
        self.terminated = false;
      }

      let Some(ending) = ending(bytes, &mut index) else {
        break;
      };

      // A prefix is written only ever immediately before something, which is what keeps a
      // chunk that ends in a newline from leaving a bare prefix on a line that never
      // receives content. An empty line still gets one: it belongs to a script too.
      if ending != CARRIAGE_RETURN {
        self.open(out, prefix)?;
      }

      out.write_all(ending)?;
      self.needs_prefix = true;
      self.terminated = ending != CARRIAGE_RETURN;
    }

    Ok(())
  }

  fn open(&mut self, out: &mut impl Write, prefix: &str) -> io::Result<()> {
    if !self.needs_prefix {
      return Ok(());
    }

    out.write_all(prefix.as_bytes())?;
    self.needs_prefix = false;

    Ok(())
  }
}

const CARRIAGE_RETURN: &[u8] = b"\r";

/// The line ending at `index`, if that is where the scan stopped, advancing past it.
///
/// `\r\n` is one ending rather than a carriage return followed by a newline, so a Windows
/// child never gets a prefix wedged between the two bytes.
fn ending<'a>(bytes: &'a [u8], index: &mut usize) -> Option<&'a [u8]> {
  let byte = *bytes.get(*index)?;

  if byte == b'\r' && bytes.get(*index + 1) == Some(&b'\n') {
    *index += 2;
    return Some(b"\r\n");
  }

  *index += 1;
  Some(if byte == b'\r' { CARRIAGE_RETURN } else { b"\n" })
}

#[cfg(test)]
mod tests {
  use super::{Multiplexer, ScriptId, Stream};
  use crate::color::{ColorLevel, Palette};

  /// One scripted write.
  type Write<'a> = (ScriptId, Stream, &'a [u8]);

  /// Runs a table of writes through an interleaved writer and gives back the exact bytes.
  fn interleaved(names: &[&str], writes: &[Write<'_>]) -> Vec<u8> {
    let mut writer = Multiplexer::interleaved(Vec::new(), Palette::new(names, ColorLevel::None));

    for (script, stream, bytes) in writes {
      writer.write(*script, *stream, bytes).expect("a Vec never fails to accept bytes");
    }

    writer.into_inner()
  }

  /// The same, as text, for the cases with nothing exotic in them.
  fn rendered(names: &[&str], writes: &[Write<'_>]) -> String {
    String::from_utf8(interleaved(names, writes)).expect("the fixtures are all UTF-8")
  }

  const OUT: Stream = Stream::Stdout;
  const ERR: Stream = Stream::Stderr;

  /// Test 5b.1 — the whole rule in one table. The exact bytes are the deliverable here,
  /// which is why this one is a snapshot and the internal state never is.
  #[test]
  fn every_line_carries_the_prefix_of_the_script_that_wrote_it() {
    let output = rendered(
      &["lint", "test"],
      &[
        (0, OUT, b"checking\n"),
        (1, OUT, b"1 passed\n2 passed\n"),
        (0, OUT, b"clean\n"),
        (1, ERR, b"one warning\n"),
      ],
    );

    insta::with_settings!({ description => "two scripts, whole lines, both streams" }, {
      insta::assert_snapshot!(output);
    });
  }

  /// Test 5b.2 — the rule that keeps two scripts off one line.
  #[test]
  fn a_switch_mid_line_gets_a_newline_before_the_new_prefix() {
    let output = rendered(&["a", "b"], &[(0, OUT, b"foo"), (1, OUT, b"bar\n")]);

    assert_eq!(output, "[a] foo\n[b] bar\n");
  }

  /// The other half of 5b.2, and the one a naive implementation breaks: a script carrying
  /// on with its own line gets neither an injected newline nor a second prefix.
  #[test]
  fn a_script_continuing_its_own_line_gets_no_second_prefix() {
    let output = rendered(&["a"], &[(0, OUT, b"foo"), (0, OUT, b"bar\n")]);

    assert_eq!(output, "[a] foobar\n");
  }

  /// Test 5b.6 — a script's own two streams follow the same rule as two scripts do.
  #[test]
  fn a_scripts_two_streams_are_both_prefixed_and_never_spliced() {
    let output = rendered(&["a"], &[(0, OUT, b"working"), (0, ERR, b"failed\n")]);

    assert_eq!(output, "[a] working\n[a] failed\n");
  }

  /// Test 5b.3 — a newline inside a chunk starts a new prefixed line, and the one that
  /// ends the chunk does not.
  #[test]
  fn newlines_inside_a_chunk_are_re_prefixed_and_the_last_one_is_not() {
    let output = rendered(&["a"], &[(0, OUT, b"one\ntwo\nthree\n")]);

    assert_eq!(output, "[a] one\n[a] two\n[a] three\n");
  }

  /// The two chunks that catch a writer emitting a prefix without checking whether
  /// anything follows it.
  #[test]
  fn an_empty_chunk_writes_nothing_and_a_lone_newline_writes_an_empty_line() {
    assert_eq!(rendered(&["a"], &[(0, OUT, b"")]), "");
    assert_eq!(rendered(&["a"], &[(0, OUT, b"one\n"), (0, OUT, b"\n")]), "[a] one\n[a] \n");
  }

  /// Test 5b.4 — the gap the reference implementation has.
  ///
  /// A chunk boundary falling inside a character is invisible here by construction: no
  /// byte of a multi-byte character can be a newline or a carriage return, so nothing
  /// this writer looks at can fall inside one.
  #[test]
  fn a_character_split_across_two_chunks_arrives_intact() {
    // The three bytes of `→`, cut after the first.
    let output = interleaved(&["a"], &[(0, OUT, &[0xE2]), (0, OUT, &[0x86, 0x92, b'\n'])]);

    assert_eq!(String::from_utf8(output).expect("valid UTF-8"), "[a] →\n");
  }

  /// Test 5b.5, first case — a carriage return passes through and the prefix comes back,
  /// so a child repainting its line still produces an attributable one.
  #[test]
  fn a_bare_carriage_return_passes_through_and_re_emits_the_prefix() {
    let output = rendered(&["a"], &[(0, OUT, b"50%\r60%\n")]);

    assert_eq!(output, "[a] 50%\r[a] 60%\n");
  }

  /// Test 5b.5, second case — one line ending, not two bytes to wedge a prefix between.
  #[test]
  fn a_carriage_return_and_newline_pair_is_one_line_ending() {
    let output = rendered(&["a"], &[(0, OUT, b"done\r\nnext\r\n")]);

    assert_eq!(output, "[a] done\r\n[a] next\r\n");
  }

  /// Test 5b.5, third case — the child means to write more on that line, so another
  /// script writing must still inject a newline first.
  #[test]
  fn a_chunk_ending_in_a_carriage_return_counts_as_unterminated() {
    let output = rendered(&["a", "b"], &[(0, OUT, b"50%\r"), (1, OUT, b"hello\n")]);

    assert_eq!(output, "[a] 50%\r\n[b] hello\n");
  }

  /// Test 5b.5, fourth case, and design D5 in one assertion: rune adds prefixes and the
  /// newlines that keep lines attributable, and changes nothing else. The reference
  /// implementation rewrites one character here; rune does not, because faithful
  /// pass-through is the promise the whole tool is sold on.
  #[test]
  fn escape_sequences_and_unusual_characters_reach_the_output_unaltered() {
    let payload = "\u{1b}[31mred\u{1b}[0m … ✓ 🎉\n";

    let output = rendered(&["a"], &[(0, OUT, payload.as_bytes())]);

    assert_eq!(output, format!("[a] {payload}"));
  }

  /// Test 5b.9 — one contiguous block per script, in the order they finished.
  ///
  /// Interleaved is right for a terminal and unreadable in a CI log, where nobody is
  /// watching it arrive and everybody is scrolling back through it afterwards.
  #[test]
  fn grouped_mode_flushes_each_script_as_one_block_in_completion_order() {
    let mut writer = Multiplexer::grouped(Vec::new(), Palette::new(&["a", "b"], ColorLevel::None));

    for (script, stream, bytes) in [
      (0, OUT, b"a1\n".as_slice()),
      (1, OUT, b"b1\n".as_slice()),
      (0, OUT, b"a2\n".as_slice()),
      (1, OUT, b"b2\n".as_slice()),
    ] {
      writer.write(script, stream, bytes).expect("a Vec never fails to accept bytes");
    }

    writer.finished(1).expect("flush b");
    writer.finished(0).expect("flush a");

    let output = String::from_utf8(writer.into_inner()).expect("valid UTF-8");
    assert_eq!(output, "[b] b1\n[b] b2\n[a] a1\n[a] a2\n");
  }

  /// A script that never reported finishing still had its output collected, and losing it
  /// would hide the reason the run ended.
  #[test]
  fn grouped_mode_writes_out_what_an_unfinished_script_managed_to_say() {
    let mut writer = Multiplexer::grouped(Vec::new(), Palette::new(&["a"], ColorLevel::None));

    writer.write(0, OUT, b"half a thought").expect("a Vec never fails to accept bytes");
    writer.finish_all().expect("flush");

    assert_eq!(String::from_utf8(writer.into_inner()).expect("valid UTF-8"), "[a] half a thought");
  }
}
