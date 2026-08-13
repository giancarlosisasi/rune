//! Strip typescript types from a config source, producing runnable Javascript

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::{ImportDeclarationSpecifier, ImportExpression, Program, Statement};
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

/// One problem found while stripping types, mapped from an oxc diagnostic
#[derive(Debug)]
pub struct StripError {
  pub message: String,
  /// Where in the file the diagnostic points, when it points anywhere. A diagnostic rune
  /// raises about the file as a whole carries none.
  pub position: Option<Position>,
}

/// A place in the TypeScript the user wrote, and the line that is there.
///
/// The parser reads the file before anything is erased, so these need no remapping — the
/// difference between this half and a position the running engine reports.
#[derive(Debug)]
pub struct Position {
  pub line: usize,
  pub column: usize,
  /// The whole line, as written, so the message can show what it is pointing at.
  pub text: String,
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
  let map = map.ok_or_else(|| {
    vec![StripError { message: "codegen produced no source map".to_owned(), position: None }]
  })?;
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
    return Err(to_strip_errors(parser_return.diagnostics, source));
  }
  let mut program = parser_return.program;

  // Rejected here, before anything runs, because the reason is static: a computed
  // specifier cannot be found by the import walk, so it would escape the hashed
  // closure and leave the cache describing a file set that is not the one used.
  if let Some(count) = count_dynamic_imports(&program) {
    return Err(vec![StripError {
      message: format!(
        "dynamic `import()` is not supported in a rune config ({count} found)\n\n\
         rune hashes every file a config imports to decide whether a cached result is \
         still valid. A specifier computed at runtime cannot be found by that walk, so \
         allowing it would mean silently serving stale configuration.\n\n\
         use a static `import` at the top of the file instead."
      ),
      position: None,
    }]);
  }

  // 2. Semantic resolve scopes and symbols.
  //
  // `new_compiler` rather than `new`, plus enum evaluation: without both, a TypeScript
  // `enum` member reaches codegen with no value and the config silently loses it.
  let semantic_return = SemanticBuilder::new_compiler().with_enum_eval(true).build(&program);
  if !semantic_return.diagnostics.is_empty() {
    return Err(to_strip_errors(semantic_return.diagnostics, source));
  }
  let scoping = semantic_return.semantic.into_scoping();

  // 3. Transform: erase TS types in place
  let options = TransformOptions::default();
  let transformer_return =
    Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
  if !transformer_return.diagnostics.is_empty() {
    return Err(to_strip_errors(transformer_return.diagnostics, source));
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

fn to_strip_errors(
  diagnostics: impl IntoIterator<Item = OxcDiagnostic>,
  source: &str,
) -> Vec<StripError> {
  diagnostics
    .into_iter()
    .map(|diagnostic| StripError {
      message: diagnostic.to_string(),
      position: first_label(&diagnostic).map(|offset| position_of(source, offset)),
    })
    .collect()
}

/// Where the diagnostic says the problem is, when it says.
///
/// A diagnostic carries its labels as byte offsets into the source that was parsed, and
/// the first one is what the message is about; any others are the context around it.
fn first_label(diagnostic: &OxcDiagnostic) -> Option<usize> {
  diagnostic.labels.first().map(|label| label.offset() as usize)
}

/// Turns a byte offset into the line and column a person counts, plus the line itself.
fn position_of(source: &str, offset: usize) -> Position {
  let offset = (0..=offset.min(source.len())).rev().find(|&at| source.is_char_boundary(at));
  let offset = offset.unwrap_or(0);

  let before = &source[..offset];
  let start = before.rfind('\n').map_or(0, |newline| newline + 1);

  Position {
    line: before.matches('\n').count() + 1,
    column: source[start..offset].chars().count() + 1,
    text: source[start..].lines().next().unwrap_or_default().to_owned(),
  }
}

/// The specifiers `source` brings `name` into scope from — a plain import, a renamed one,
/// or a re-export.
///
/// Read on a failure path only, to say which file wrote an import the engine refused. The
/// question is about the text the user wrote, so this parses and stops: no semantic pass,
/// no transform, no codegen.
pub fn specifiers_importing(source: &str, name: &str) -> Vec<String> {
  let allocator = Allocator::default();
  let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();

  parsed
    .program
    .body
    .iter()
    .filter_map(|statement| match statement {
      Statement::ImportDeclaration(declaration) => {
        let specifiers = declaration.specifiers.as_ref()?;
        specifiers
          .iter()
          .any(|specifier| binds(specifier, name))
          .then(|| declaration.source.value.to_string())
      }
      Statement::ExportNamedDeclaration(declaration) => {
        let from = declaration.source.as_ref()?;
        declaration
          .specifiers
          .iter()
          .any(|specifier| specifier.local.name() == name)
          .then(|| from.value.to_string())
      }
      // `export *` passes on whatever it is given, so any name may have come through it.
      Statement::ExportAllDeclaration(declaration) => Some(declaration.source.value.to_string()),
      _ => None,
    })
    .collect()
}

fn binds(specifier: &ImportDeclarationSpecifier<'_>, name: &str) -> bool {
  match specifier {
    ImportDeclarationSpecifier::ImportSpecifier(one) => one.imported.name() == name,
    ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => name == "default",
    // A namespace import asks for the whole module, never for one name in it.
    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => false,
  }
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

  use super::{StrippedJs, specifiers_importing, strip_types};

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
            position: Some(
                Position {
                    line: 1,
                    column: 18,
                    text: "const x: number =",
                },
            ),
        },
    ]
    "#);
  }

  /// The span the parser hands over is a byte offset, and a person counts lines and
  /// characters. The line under the mistake is what makes the count checkable by eye.
  #[test]
  fn a_diagnostic_on_a_later_line_reports_that_line() {
    let source = "export const a = 1;\nexport const b = 2;\nexport const broken = {;\n";
    let errors = strip_types(source, Path::new("rune.config.ts")).unwrap_err();
    let position = errors[0].position.as_ref().expect("a syntax error points somewhere");

    assert_eq!(position.line, 3);
    assert_eq!(position.text, "export const broken = {;");
  }

  /// A character outside ASCII is one column, not the two or three bytes it occupies.
  #[test]
  fn a_column_counts_characters_rather_than_bytes() {
    let errors = strip_types("const é = { ;\n", Path::new("rune.config.ts")).unwrap_err();
    let position = errors[0].position.as_ref().expect("a syntax error points somewhere");

    assert_eq!(position.column, 13);
  }

  /// The failure path that finds who wrote an import the engine refused.
  #[test]
  fn an_imported_name_is_traced_back_to_its_specifier() {
    let source = "import { rune, defineConfig as make } from '@gio-labs/rune';\n\
                  import other from './other';\n\
                  export { helper } from './helpers';\n";

    assert_eq!(specifiers_importing(source, "defineConfig"), ["@gio-labs/rune"]);
    assert_eq!(specifiers_importing(source, "default"), ["./other"]);
    assert_eq!(specifiers_importing(source, "helper"), ["./helpers"]);
    assert!(specifiers_importing(source, "absent").is_empty());
  }
}
