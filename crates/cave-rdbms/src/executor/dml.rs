//! DML helpers (project, filter, sort, group).

use crate::sql::ast::{Expr, OrderBy};
use crate::storage::schema::Row;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dml_module_exists() {
        let _row: Row = vec![];
    }
}
