//! Partition spec evolution.
//!
//! Iceberg supports rewriting a table's partition spec without rewriting the
//! data. Each spec has a unique `spec_id`; entries from older specs remain
//! readable but pruning/predicate-pushdown uses the matching spec for each
//! file. See <https://iceberg.apache.org/spec/#partition-evolution>.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EvolutionError {
    #[error("spec already exists: {0}")]
    SpecExists(i32),
    #[error("spec not found: {0}")]
    SpecNotFound(i32),
    #[error("transform not allowed for column type: {0}")]
    InvalidTransform(String),
    #[error("cannot drop default spec")]
    CannotDropDefault,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Transform {
    Identity,
    Bucket(u32),
    Truncate(u32),
    Year,
    Month,
    Day,
    Hour,
    Void,
}

impl Transform {
    pub fn name(&self) -> String {
        match self {
            Self::Identity => "identity".into(),
            Self::Bucket(n) => format!("bucket[{}]", n),
            Self::Truncate(n) => format!("truncate[{}]", n),
            Self::Year => "year".into(),
            Self::Month => "month".into(),
            Self::Day => "day".into(),
            Self::Hour => "hour".into(),
            Self::Void => "void".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartitionField {
    pub source_id: i32,
    pub field_id: i32,
    pub name: String,
    pub transform: Transform,
}

#[derive(Debug, Clone)]
pub struct PartitionSpec {
    pub spec_id: i32,
    pub fields: Vec<PartitionField>,
}

#[derive(Debug, Default)]
pub struct PartitionRegistry {
    specs: BTreeMap<i32, PartitionSpec>,
    default_spec_id: i32,
    next_field_id: i32,
}

impl PartitionRegistry {
    pub fn new() -> Self {
        Self {
            specs: BTreeMap::new(),
            default_spec_id: 0,
            next_field_id: 1000,
        }
    }

    pub fn register(&mut self, spec: PartitionSpec) -> Result<(), EvolutionError> {
        if self.specs.contains_key(&spec.spec_id) {
            return Err(EvolutionError::SpecExists(spec.spec_id));
        }
        if self.specs.is_empty() {
            self.default_spec_id = spec.spec_id;
        }
        self.specs.insert(spec.spec_id, spec);
        Ok(())
    }

    pub fn next_field_id(&mut self) -> i32 {
        let id = self.next_field_id;
        self.next_field_id += 1;
        id
    }

    pub fn evolve_add_field(
        &mut self,
        from_spec: i32,
        new_spec_id: i32,
        field: PartitionField,
    ) -> Result<(), EvolutionError> {
        let base = self
            .specs
            .get(&from_spec)
            .ok_or(EvolutionError::SpecNotFound(from_spec))?
            .clone();
        let mut new_fields = base.fields;
        new_fields.push(field);
        let new_spec = PartitionSpec {
            spec_id: new_spec_id,
            fields: new_fields,
        };
        self.register(new_spec)
    }

    pub fn evolve_drop_field(
        &mut self,
        from_spec: i32,
        new_spec_id: i32,
        field_id: i32,
    ) -> Result<(), EvolutionError> {
        let base = self
            .specs
            .get(&from_spec)
            .ok_or(EvolutionError::SpecNotFound(from_spec))?
            .clone();
        let mut new_fields = base.fields;
        // Iceberg: dropped fields are kept with `Transform::Void` to preserve metadata.
        for f in new_fields.iter_mut() {
            if f.field_id == field_id {
                f.transform = Transform::Void;
            }
        }
        let new_spec = PartitionSpec {
            spec_id: new_spec_id,
            fields: new_fields,
        };
        self.register(new_spec)
    }

    pub fn set_default(&mut self, spec_id: i32) -> Result<(), EvolutionError> {
        if !self.specs.contains_key(&spec_id) {
            return Err(EvolutionError::SpecNotFound(spec_id));
        }
        self.default_spec_id = spec_id;
        Ok(())
    }

    pub fn drop_spec(&mut self, spec_id: i32) -> Result<(), EvolutionError> {
        if spec_id == self.default_spec_id {
            return Err(EvolutionError::CannotDropDefault);
        }
        self.specs
            .remove(&spec_id)
            .ok_or(EvolutionError::SpecNotFound(spec_id))?;
        Ok(())
    }

    pub fn get(&self, spec_id: i32) -> Option<&PartitionSpec> {
        self.specs.get(&spec_id)
    }

    pub fn default_spec(&self) -> Option<&PartitionSpec> {
        self.specs.get(&self.default_spec_id)
    }

    pub fn count(&self) -> usize {
        self.specs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(id: i32, name: &str, transform: Transform) -> PartitionField {
        PartitionField {
            source_id: 1,
            field_id: id,
            name: name.into(),
            transform,
        }
    }

    #[test]
    fn register_first_spec_sets_default() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![field(1000, "year", Transform::Year)],
        })
        .unwrap();
        assert_eq!(r.default_spec().unwrap().spec_id, 0);
    }

    #[test]
    fn register_duplicate_errors() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![],
        })
        .unwrap();
        let err = r
            .register(PartitionSpec {
                spec_id: 0,
                fields: vec![],
            })
            .unwrap_err();
        assert_eq!(err, EvolutionError::SpecExists(0));
    }

    #[test]
    fn evolve_adds_field_under_new_spec_id() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![field(1000, "year", Transform::Year)],
        })
        .unwrap();
        r.evolve_add_field(0, 1, field(1001, "month", Transform::Month))
            .unwrap();
        assert_eq!(r.get(1).unwrap().fields.len(), 2);
        assert_eq!(r.get(0).unwrap().fields.len(), 1);
    }

    #[test]
    fn evolve_drop_field_replaces_with_void() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![
                field(1000, "year", Transform::Year),
                field(1001, "month", Transform::Month),
            ],
        })
        .unwrap();
        r.evolve_drop_field(0, 1, 1001).unwrap();
        let f = &r.get(1).unwrap().fields[1];
        assert_eq!(f.transform, Transform::Void);
    }

    #[test]
    fn set_default_changes_default_spec() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![],
        })
        .unwrap();
        r.register(PartitionSpec {
            spec_id: 1,
            fields: vec![],
        })
        .unwrap();
        r.set_default(1).unwrap();
        assert_eq!(r.default_spec().unwrap().spec_id, 1);
    }

    #[test]
    fn set_default_unknown_errors() {
        let mut r = PartitionRegistry::new();
        let err = r.set_default(99).unwrap_err();
        assert_eq!(err, EvolutionError::SpecNotFound(99));
    }

    #[test]
    fn drop_default_errors() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![],
        })
        .unwrap();
        let err = r.drop_spec(0).unwrap_err();
        assert_eq!(err, EvolutionError::CannotDropDefault);
    }

    #[test]
    fn drop_non_default_succeeds() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![],
        })
        .unwrap();
        r.register(PartitionSpec {
            spec_id: 1,
            fields: vec![],
        })
        .unwrap();
        r.drop_spec(1).unwrap();
        assert!(r.get(1).is_none());
    }

    #[test]
    fn next_field_id_starts_at_1000_and_increments() {
        let mut r = PartitionRegistry::new();
        let a = r.next_field_id();
        let b = r.next_field_id();
        assert_eq!(a, 1000);
        assert_eq!(b, 1001);
    }

    #[test]
    fn transform_names_match_spec() {
        assert_eq!(Transform::Identity.name(), "identity");
        assert_eq!(Transform::Year.name(), "year");
        assert_eq!(Transform::Bucket(16).name(), "bucket[16]");
        assert_eq!(Transform::Truncate(8).name(), "truncate[8]");
        assert_eq!(Transform::Void.name(), "void");
    }

    #[test]
    fn evolve_from_unknown_spec_errors() {
        let mut r = PartitionRegistry::new();
        let err = r
            .evolve_add_field(99, 1, field(1000, "x", Transform::Identity))
            .unwrap_err();
        assert_eq!(err, EvolutionError::SpecNotFound(99));
    }

    #[test]
    fn count_reflects_registered_specs() {
        let mut r = PartitionRegistry::new();
        r.register(PartitionSpec {
            spec_id: 0,
            fields: vec![],
        })
        .unwrap();
        r.register(PartitionSpec {
            spec_id: 1,
            fields: vec![],
        })
        .unwrap();
        assert_eq!(r.count(), 2);
    }
}
