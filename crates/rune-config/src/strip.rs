//! Strip typescript types from a config source, producing runnable Javascript

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::{ImportExpression, Program, Statement};
use oxc::ast_visit::Visit;
use oxc::codegen::{Codegen, CodegenOptions};
use oxc::diagnostics::OxcDiagnostic;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};
use oxc_sourcemap::SourceMap;

use crate::resolve::is_relative;

/// Javascript emitted after erasing the Typescript types from a config file
#[derive(Debug)]
pub struct StrippedJs {
  /// The generated Javascript - non-minified, ready for QuickJS to evaluate.
  pub code: String,
  /// Relative import specifiers this module needs at runtime (`./helper`, `../env`).
  /// Feeds the module loader and the cache-key closure. Bare/npm imports are excluded
  pub imports: Vec<String>,
}

/// One problem found while stripping types, mapped from an exc diagnostic
#[derive(Debug)]
pub struct StripError {
  pub message: String,
}

/// Strip the typescript types from `source`, returning plain javascript
///
/// `path` names the file for diagnostics; the source is always parsed as Typescript
pub fn strip_types(source: &str, path: &Path) -> Result<StrippedJs, Vec<StripError>> {
  strip(source, path, false).map(|(stripped, _)| stripped)
}

/// The same strip, keeping the map from generated positions back to the `.ts` source.
///
/// Stripping is transform-then-reprint, so a generated line number is not the line the
/// user wrote. Only a caller that has to report a position pays for the map, which is
/// why this is a second entry point and not an extra field on the common one.
pub fn strip_types_with_map(
  source: &str,
  path: &Path,
) -> Result<(StrippedJs, SourceMap<'static>), Vec<StripError>> {
  let (stripped, map) = strip(source, path, true)?;
  let map =
    map.ok_or_else(|| vec![StripError { message: "codegen produced no source map".to_owned() }])?;
  Ok((stripped, map))
}

fn strip(
  source: &str,
  path: &Path,
  want_map: bool,
) -> Result<(StrippedJs, Option<SourceMap<'static>>), Vec<StripError>> {
  let allocator = Allocator::default();
  let source_type = SourceType::ts();

  // 1. Parse: text -> AST
  let parser_return = Parser::new(&allocator, source, source_type).parse();
  if parser_return.panicked || !parser_return.diagnostics.is_empty() {
    return Err(to_strip_errors(parser_return.diagnostics));
  }
  let mut program = parser_return.program;

  // Rejected here, before anything runs, because the reason is static: a computed
  // specifier cannot be found by the import walk, so it would escape the hashed
  // closure and leave the cache describing a file set that is not the one used.
  if let Some(count) = count_dynamic_imports(&program) {
    return Err(vec![StripError {
      message: format!(
        "dynamic `import()` is not supported in a rune config ({count} found in {})\n\n\
         rune hashes every file a config imports to decide whether a cached result is \
         still valid. A specifier computed at runtime cannot be found by that walk, so \
         allowing it would mean silently serving stale configuration.\n\n\
         use a static `import` at the top of the file instead.",
        path.display()
      ),
    }]);
  }

  // 2. Semantic resolve scopes and symbols.
  //
  // `new_compiler` rather than `new`, plus enum evaluation: without both, a TypeScript
  // `enum` member reaches codegen with no value and the config silently loses it.
  let semantic_return = SemanticBuilder::new_compiler().with_enum_eval(true).build(&program);
  if !semantic_return.diagnostics.is_empty() {
    return Err(to_strip_errors(semantic_return.diagnostics));
  }
  let scoping = semantic_return.semantic.into_scoping();

  // 3. Transform: erase TS types in place
  let options = TransformOptions::default();
  let transformer_return =
    Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
  if !transformer_return.diagnostics.is_empty() {
    return Err(to_strip_errors(transformer_return.diagnostics));
  }

  // 4. Codegen: AST -> Javascript string
  let options = CodegenOptions {
    source_map_path: want_map.then(|| path.to_owned()),
    ..CodegenOptions::default()
  };
  let codegen_return = Codegen::new().with_options(options).build(&program);
  let map = codegen_return.map.map(SourceMap::into_owned);

  // 5. Collect the relative files this module imports (same AST - no second parser)
  let imports = collect_relative_imports(&program);

  Ok((StrippedJs { code: codegen_return.code, imports }, map))
}

fn to_strip_errors(diagnostics: impl IntoIterator<Item = OxcDiagnostic>) -> Vec<StripError> {
  diagnostics.into_iter().map(|diagnostic| StripError { message: diagnostic.to_string() }).collect()
}

fn collect_relative_imports(program: &Program<'_>) -> Vec<String> {
  program
    .body
    .iter()
    .filter_map(|statement| match statement {
      Statement::ImportDeclaration(decl) => Some(decl.source.value.as_str()),
      Statement::ExportNamedDeclaration(decl) => decl.source.as_ref().map(|s| s.value.as_str()),
      Statement::ExportAllDeclaration(decl) => Some(decl.source.value.as_str()),
      _ => None,
    })
    .filter(|&specifier| is_relative(specifier))
    .map(String::from)
    .collect()
}

/// Counts `import(...)` expressions anywhere in the program, not only at the top level —
/// they nest inside any expression, so a statement scan would miss most of them.
fn count_dynamic_imports(program: &Program<'_>) -> Option<usize> {
  let mut finder = DynamicImportFinder { count: 0 };
  finder.visit_program(program);
  (finder.count > 0).then_some(finder.count)
}

struct DynamicImportFinder {
  count: usize,
}

impl<'a> Visit<'a> for DynamicImportFinder {
  fn visit_import_expression(&mut self, _expression: &ImportExpression<'a>) {
    self.count += 1;
  }
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  use insta::{assert_debug_snapshot, assert_snapshot};

  use super::{StrippedJs, strip_types};

  fn strip(source: &str) -> StrippedJs {
    strip_types(source, Path::new("rune.config.ts")).expect("should strip cleanly")
  }

  #[test]
  fn strips_a_plain_ts_config() {
    let stripped = strip("const answer: number = 42;\nexport default { answer };\n");
    assert_snapshot!(stripped.code, @"
    const answer = 42;
    export default { answer };
    ");
  }

  #[test]
  fn erases_type_only_imports() {
    let source = "import type { Cfg } from './types';\nimport { helper } from './helper';\nexport default { value: helper };\n";
    let stripped = strip(source);
    assert_snapshot!(stripped.code, @r#"
    import { helper } from "./helper";
    export default { value: helper };
    "#);
    assert_debug_snapshot!(stripped.imports, @r#"
    [
        "./helper",
    ]
    "#);
  }

  #[test]
  fn reports_syntax_error_diagnostics() {
    let errors = strip_types("const x: number =", Path::new("rune.config.ts")).unwrap_err();
    assert_debug_snapshot!(errors, @r#"
    [
        StripError {
            message: "Unexpected token",
        },
    ]
    "#);
  }
}
