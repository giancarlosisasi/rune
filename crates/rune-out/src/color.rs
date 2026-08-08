//! How much color rune may use, and what it tells a child about color.
//!
//! Detection happens here, at the edge, and everything downstream takes the level as an
//! argument. A component that asked the terminal itself would behave one way on a laptop
//! and another on a CI runner, and no test could pin either.

use anstyle::{AnsiColor, Reset, Style};

/// How much color the stream rune writes to supports.
///
/// The names map one for one onto the values `FORCE_COLOR` takes, because the level rune
/// detects for itself is exactly the level it hands a piped child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorLevel {
  None,
  Basic,
  Ansi256,
  TrueColor,
}

/// The variable a piped child reads to decide whether to emit color at all.
pub const COLOR_VARIABLE: &str = "FORCE_COLOR";

impl ColorLevel {
  /// The `FORCE_COLOR` value standing for this level.
  pub fn indicator(self) -> &'static str {
    match self {
      Self::None => "0",
      Self::Basic => "1",
      Self::Ansi256 => "2",
      Self::TrueColor => "3",
    }
  }

  /// What rune's own standard output actually supports.
  ///
  /// `NO_COLOR`, `FORCE_COLOR`, `TERM` and the CI variables are all read by the crate
  /// behind this, which is a port of the same detection the JavaScript tools use. One
  /// answer for rune's prefixes and for its children beats two that disagree.
  pub fn detect() -> Self {
    // `0` is what rune itself writes into `FORCE_COLOR` to tell a child "no color", so it
    // has to mean the same thing when rune reads it. The crate behind this reads it only
    // as "nothing forced", which under a terminal leaves rune coloring its own prefixes
    // while telling its children not to color theirs.
    if std::env::var(COLOR_VARIABLE).is_ok_and(|level| level == "0" || level == "false") {
      return Self::None;
    }

    let Some(support) = supports_color::on(supports_color::Stream::Stdout) else {
      return Self::None;
    };

    if support.has_16m {
      Self::TrueColor
    } else if support.has_256 {
      Self::Ansi256
    } else if support.has_basic {
      Self::Basic
    } else {
      Self::None
    }
  }
}

/// What rune should set `FORCE_COLOR` to for a piped child, or nothing at all.
///
/// A piped child sees no terminal and turns its own color off, which is why the indicator
/// is injected. Two cases leave it alone: rune's own output has no color to forward, or
/// the user already said what they wanted and it is not rune's place to argue.
pub fn forced_color(level: ColorLevel, already_set: Option<&str>) -> Option<&'static str> {
  if already_set.is_some() || level == ColorLevel::None {
    return None;
  }

  Some(level.indicator())
}

/// The colors prefixes cycle through.
///
/// Red is left out: a red prefix on an ordinary line reads as a failure the script never
/// reported. The rest are picked to stay apart on both dark and light backgrounds.
const WHEEL: [AnsiColor; 6] = [
  AnsiColor::Cyan,
  AnsiColor::Magenta,
  AnsiColor::Green,
  AnsiColor::Yellow,
  AnsiColor::Blue,
  AnsiColor::BrightMagenta,
];

/// The prefix each script of one run writes, rendered once and stable for the whole run.
///
/// Stability is the point. A prefix whose color moved between writes would be useless for
/// the one thing it exists for, which is scanning a log for one script's lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
  prefixes: Vec<String>,
}

impl Palette {
  pub fn new(names: &[&str], level: ColorLevel) -> Self {
    let prefixes =
      names.iter().enumerate().map(|(index, name)| render(name, color_for(index, level))).collect();

    Self { prefixes }
  }

  /// The prefix for one script. Scripts are numbered by their position in the run.
  pub fn prefix(&self, script: usize) -> &str {
    &self.prefixes[script]
  }

  pub fn len(&self) -> usize {
    self.prefixes.len()
  }

  pub fn is_empty(&self) -> bool {
    self.prefixes.is_empty()
  }
}

/// A level of `None` is what `NO_COLOR` becomes by the time it reaches here, and it takes
/// the color out of rune's own prefixes only. The child's bytes are never touched.
fn color_for(index: usize, level: ColorLevel) -> Option<AnsiColor> {
  if level == ColorLevel::None {
    return None;
  }

  Some(WHEEL[index % WHEEL.len()])
}

fn render(name: &str, color: Option<AnsiColor>) -> String {
  let Some(color) = color else {
    return format!("[{name}] ");
  };

  let style = Style::new().fg_color(Some(color.into()));

  format!("{style}[{name}]{} ", Reset.render())
}

#[cfg(test)]
mod tests {
  use super::{ColorLevel, Palette, forced_color};

  /// Test 5b.7's first half — the same script draws the same prefix every time, and two
  /// scripts do not draw the same one.
  #[test]
  fn a_scripts_color_is_the_same_every_time_it_is_asked_for() {
    let palette = Palette::new(&["lint", "test"], ColorLevel::Basic);

    assert_eq!(palette.prefix(0), palette.prefix(0));
    assert_ne!(palette.prefix(0), palette.prefix(1));
    assert!(palette.prefix(0).contains("[lint]"));
  }

  /// Test 5b.7's second half — no color means no color codes anywhere in rune's own
  /// prefix. The child's bytes are a separate question, and the answer there is never.
  #[test]
  fn no_color_leaves_the_prefix_plain() {
    let palette = Palette::new(&["lint"], ColorLevel::None);

    assert_eq!(palette.prefix(0), "[lint] ");
  }

  #[test]
  fn a_run_with_more_scripts_than_colors_still_names_every_one() {
    let names: Vec<String> = (0..9).map(|index| format!("s{index}")).collect();
    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();

    let palette = Palette::new(&borrowed, ColorLevel::Basic);

    assert_eq!(palette.len(), 9);
    assert!(palette.prefix(8).contains("[s8]"));
  }

  /// Test 5b.8, all three cases. The third is the one worth writing down: concurrently
  /// spreads the real environment over its own value precisely so a user can override it.
  #[test]
  fn the_color_indicator_is_injected_only_when_it_helps() {
    assert_eq!(forced_color(ColorLevel::Ansi256, None), Some("2"));
    assert_eq!(forced_color(ColorLevel::None, None), None, "nothing to forward");
    assert_eq!(forced_color(ColorLevel::TrueColor, Some("0")), None, "the user decides");
  }

  /// The value rune writes and the value rune reads have to mean the same thing. They did
  /// not: under a terminal, `FORCE_COLOR=0` left rune coloring its own prefixes while
  /// handing every child the same `0` and telling it not to.
  ///
  /// Both spellings in one test because the variable is process-wide: the workspace runs
  /// each test in a process of its own, which is what makes writing it safe at all.
  #[test]
  fn a_force_color_of_zero_turns_runes_own_color_off() {
    for spelling in ["0", "false"] {
      // SAFETY: nextest gives every test its own process, so no other thread is reading
      // the environment while this writes it.
      unsafe { std::env::set_var(super::COLOR_VARIABLE, spelling) };
      assert_eq!(ColorLevel::detect(), ColorLevel::None, "`{spelling}` must mean no color");
    }
  }

  #[test]
  fn every_level_has_the_indicator_a_child_expects() {
    assert_eq!(ColorLevel::None.indicator(), "0");
    assert_eq!(ColorLevel::Basic.indicator(), "1");
    assert_eq!(ColorLevel::Ansi256.indicator(), "2");
    assert_eq!(ColorLevel::TrueColor.indicator(), "3");
  }
}
