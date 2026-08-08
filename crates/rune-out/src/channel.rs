//! The seam between the processes producing output and the one place writing it.
//!
//! Producers hand chunks over; one consumer writes them. The queue between them is
//! bounded on purpose: writing to a terminal can be slow, a child can be fast, and an
//! unbounded queue answers that difference by growing rune's own memory until something
//! gives. Bounded, the producer waits instead — which is backpressure the child feels as
//! a full pipe, exactly as it would without rune.
//!
//! Waiting here yields rather than blocks. A producer parked on a full queue must not
//! stop the clock the rest of the run is using: a group whose output has nowhere to go
//! still has to notice a failing member and still has to finish tearing its children down.

use std::io::{self, Write};

use tokio::sync::mpsc::{Receiver, Sender, channel as bounded};

use crate::multiplex::{Multiplexer, ScriptId, Stream};

/// How many chunks may wait for the writer before a producer has to wait for it.
///
/// Large enough that an ordinary burst never blocks, small enough that the queue is a
/// buffer rather than a place output goes to live.
pub const CAPACITY: usize = 256;

/// Something that happened to one script's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
  Chunk {
    script: ScriptId,
    stream: Stream,
    bytes: Vec<u8>,
  },
  /// That script will produce nothing further. Grouped mode writes its block here.
  Finished {
    script: ScriptId,
  },
}

/// A bounded queue from any number of producers to one writer.
pub fn channel() -> (Sender<Event>, Receiver<Event>) {
  bounded(CAPACITY)
}

/// Writes every event until the last producer is gone.
///
/// Stopping at the first write failure is deliberate. The one failure that happens in
/// practice is the reader of rune's own output going away, and retrying into a closed
/// pipe produces nothing but more failures.
pub async fn pump<W: Write>(
  events: &mut Receiver<Event>,
  out: &mut Multiplexer<W>,
) -> io::Result<()> {
  while let Some(event) = events.recv().await {
    match event {
      Event::Chunk { script, stream, bytes } => out.write(script, stream, &bytes)?,
      Event::Finished { script } => out.finished(script)?,
    }

    // Per event rather than per line: a chunk that carries a prompt or a repainted
    // progress bar has no newline in it, and holding it back makes the script look hung.
    out.flush()?;
  }

  out.finish_all()
}

#[cfg(test)]
mod tests {
  use tokio::sync::mpsc::error::TrySendError;

  use super::{CAPACITY, Event, channel, pump};
  use crate::color::{ColorLevel, Palette};
  use crate::multiplex::{Multiplexer, Stream};

  fn chunk(script: usize, text: &str) -> Event {
    Event::Chunk { script, stream: Stream::Stdout, bytes: text.as_bytes().to_vec() }
  }

  /// Test 5b.10 — the bound, asserted by construction and by a send that reports it.
  ///
  /// No timing and no sleeping. "A slow writer blocks producers" is true and untestable
  /// without a clock; "the queue holds exactly this many and then says so" is the same
  /// property, stated in a way that cannot go flaky.
  #[test]
  fn the_queue_holds_a_fixed_number_and_then_reports_itself_full() {
    let (sender, _receiver) = channel();

    for index in 0..CAPACITY {
      sender.try_send(chunk(0, "x")).unwrap_or_else(|_| panic!("chunk {index} fits"));
    }

    let overflow = sender.try_send(chunk(0, "one too many"));

    assert!(
      matches!(overflow, Err(TrySendError::Full(_))),
      "the queue must refuse rather than grow"
    );
  }

  /// The consumer half of the seam, end to end with no processes anywhere near it.
  #[tokio::test]
  async fn the_pump_writes_every_event_and_stops_when_the_producers_are_gone() {
    let (sender, mut receiver) = channel();
    let mut writer = Multiplexer::grouped(Vec::new(), Palette::new(&["a", "b"], ColorLevel::None));

    for event in [
      chunk(0, "a1\n"),
      chunk(1, "b1\n"),
      Event::Finished { script: 1 },
      chunk(0, "a2\n"),
      Event::Finished { script: 0 },
    ] {
      sender.try_send(event).expect("the queue is empty and the receiver is alive");
    }
    drop(sender);

    pump(&mut receiver, &mut writer).await.expect("a Vec never fails to accept bytes");

    let output = String::from_utf8(writer.into_inner()).expect("valid UTF-8");
    assert_eq!(output, "[b] b1\n[a] a1\n[a] a2\n");
  }
}
