// SPDX-License-Identifier: AGPL-3.0-or-later
//! LogQL engine — lexer, parser, AST, and evaluator.

pub mod ast;
pub mod eval;
pub mod lexer;
pub mod parser;

pub use ast::Query;
pub use eval::Evaluator;
pub use parser::{ParseError, Parser};
