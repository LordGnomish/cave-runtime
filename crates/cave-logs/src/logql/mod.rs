//! LogQL query engine — parser + evaluator.

pub mod ast;
pub mod eval;
pub mod lexer;
pub mod parser;

pub use eval::Evaluator;
pub use parser::parse;
