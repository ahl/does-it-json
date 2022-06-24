use schema::validate_schema_object;
use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

mod schema;

#[derive(Error, Debug)]
pub enum Error {
    #[error("error serializing item")]
    SerializationError(#[from] serde_json::Error),
    #[error("invalid schema at {path}: {details}")]
    InvalidSchema { path: String, details: String },
    #[error("{value} did not conform to the schema at {path}: {details}")]
    InvalidValue {
        path: String,
        value: Value,
        details: String,
    },
}

/// Confirm that an item matches its schema.
///
/// The item's type must implement `Serialize` and `JsonSchema`. This function
/// serializes the item and compares that serialization to the type's schema.
pub fn validate<T: JsonSchema + Serialize>(item: &T) -> Result<(), Error> {
    let value = serde_json::to_value(item)?;

    let RootSchema {
        schema,
        definitions,
        ..
    } = schema_for!(T);

    validate_schema_object("$", &schema, &definitions, &value)
}

/// Confirm that an item matches its schema and print on failure.
///
/// See [`validate`].
pub fn validate_with_output<T: JsonSchema + Serialize>(item: &T) -> Result<(), String> {
    validate(item).map_err(|e| {
        let schema = schema_for!(T);
        format!(
            "error: {e}\nschema: {}\nvalue: {}",
            serde_json::to_string_pretty(&schema).unwrap(),
            serde_json::to_string_pretty(&item).unwrap(),
        )
    })
}
