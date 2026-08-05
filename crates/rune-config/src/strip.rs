//! Strip typescript types from a config source, producing runnable Javascript

use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::{Program, Statement};
use oxc::codegen::Codegen;
use oxc::diagnostics::OxcDiagnostic;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};

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
  let allocator = Allocator::default();
  let source_type = SourceType::ts();

  // 1. Parse: text -> AST
  let parser_return = Parser::new(&allocator, source, source_type).parse();
  if parser_return.panicked || !parser_return.diagnostics.is_empty() {
    return Err(to_strip_errors(parser_return.diagnostics));
  }
  let mut program = parser_return.program;

  // 2. Semantic resolve scopes and symbols
  let semantic_return = SemanticBuilder::new().build(&program);
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
  let codegen_return = Codegen::new().build(&program);

  // 5. Collect the relative files this module imports (same AST - no second parser)
  let imports = collect_relative_imports(&program);

  Ok(StrippedJs { code: codegen_return.code, imports })
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

fn is_relative(specifier: &str) -> bool {
  specifier.starts_with("./") || specifier.starts_with("../")
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
