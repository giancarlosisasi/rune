//! Configuration loading for rune: `rune.config.ts` discovery and parsing.
pub mod strip;

// #[cfg(test)]
// mod tests {
//   use rquickjs::{Context, Runtime};

//   #[test]
//   fn quickjs_evaluates_arithmetic() {
//     let runtime = Runtime::new().expect("create QuickJS runtime");
//     let context = Context::full(&runtime).expect("create context");

//     let sum: i32 = context.with(|ctx| ctx.eval("1+1").expect("evaluate 1+1"));

//     assert_eq!(sum, 2);
//   }

//   #[test]
//   fn quickjs_evaluates_a_string() {
//     let runtime = Runtime::new().expect("create QuickJS runtime");
//     let context = Context::full(&runtime).expect("create context");

//     let greeting: String =
//       context.with(|ctx| ctx.eval("'hello ' + 'rune'").expect("evaluate string concat"));

//     assert_eq!(greeting, "hello rune");
//   }
// }
