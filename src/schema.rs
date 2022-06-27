use std::collections::BTreeMap;

use regex::Regex;
use schemars::schema::{
    ArrayValidation, InstanceType, NumberValidation, ObjectValidation, Schema, SchemaObject,
    SingleOrVec, StringValidation, SubschemaValidation,
};
use serde_json::Value;

use crate::Error;
pub fn validate_schema(
    path: &str,
    schema: &Schema,
    definitions: &BTreeMap<String, Schema>,
    value: &Value,
) -> Result<(), Error> {
    match schema {
        Schema::Object(obj) => validate_schema_object(path, obj, definitions, value),
        Schema::Bool(true) => Ok(()),
        Schema::Bool(false) => Err(Error::InvalidValue {
            path: path.to_string(),
            value: value.clone(),
            details: "trying to match against the empty set schema".to_string(),
        }),
    }
}

pub fn validate_schema_object(
    path: &str,
    schema: &SchemaObject,
    definitions: &BTreeMap<String, Schema>,
    value: &Value,
) -> Result<(), Error> {
    let SchemaObject {
        instance_type,
        enum_values,
        const_value,
        subschemas,
        number,
        string,
        array,
        object,
        reference,
        ..
    } = schema;

    if let Some(instance_type) = instance_type {
        match instance_type {
            SingleOrVec::Single(s) => {
                if is_valid_instance_type(s.as_ref(), value) {
                    Ok(())
                } else {
                    Err(Error::InvalidValue {
                        path: path.to_string(),
                        value: value.clone(),
                        details: format!("value is not of type {:?}", s.as_ref()),
                    })
                }
            }
            SingleOrVec::Vec(v) => {
                if v.iter().any(|s| is_valid_instance_type(s, value)) {
                    Ok(())
                } else {
                    Err(Error::InvalidValue {
                        path: path.to_string(),
                        value: value.clone(),
                        details: format!("value is not any of {:?}", v),
                    })
                }
            }
        }?;
    }

    match (const_value, enum_values) {
        (Some(_), Some(_)) => {
            return Err(Error::InvalidSchema {
                path: path.to_string(),
                details: "both `const` and `enum` present".to_string(),
            })
        }

        (Some(const_value), None) if const_value == value => Ok(()),
        (Some(_), None) => Err(Error::InvalidValue {
            path: format!("{}.{}", path, "const"),
            value: value.clone(),
            details: "mismatch with expected const value".to_string(),
        }),

        (None, Some(enum_values)) if enum_values.contains(value) => Ok(()),
        (None, Some(_)) => Err(Error::InvalidValue {
            path: format!("{}.{}", path, "enum"),
            value: value.clone(),
            details: "not a valid enumerated value".to_string(),
        }),

        (None, None) => Ok(()),
    }?;

    if let Some(SubschemaValidation {
        all_of,
        any_of,
        one_of,
        not,
        if_schema,
        then_schema,
        else_schema,
    }) = &subschemas.as_ref().map(Box::as_ref)
    {
        if let Some(set) = all_of {
            let bad_count = set
                .iter()
                .filter(|sub_schema| {
                    validate_schema(&format!("{}.allOf", path), sub_schema, definitions, value)
                        .is_err()
                })
                .count();
            if bad_count != 0 {
                return Err(Error::InvalidValue {
                    path: format!("{}.allOf", path),
                    value: value.clone(),
                    details: format!(
                        "value did not validate for {} of {} `allOf` schemas",
                        bad_count,
                        set.len()
                    ),
                });
            }
        }

        if let Some(set) = any_of {
            if !set.iter().any(|sub_schema| {
                validate_schema(&format!("{}.anyOf", path), sub_schema, definitions, value).is_ok()
            }) {
                return Err(Error::InvalidValue {
                    path: format!("{}.anyOf", path),
                    value: value.clone(),
                    details: "value did not validate for any `anyOf` schemas".to_string(),
                });
            }
        }

        if let Some(set) = one_of {
            let good_count = set
                .iter()
                .filter(|sub_schema| {
                    validate_schema(&format!("{}.oneOf", path), sub_schema, definitions, value)
                        .is_ok()
                })
                .count();
            if good_count != 1 {
                return Err(Error::InvalidValue {
                    path: format!("{}.oneOf", path),
                    value: value.clone(),
                    details: format!(
                        "value validated against {} of {} `oneOf` schemas (rather than 1)",
                        good_count,
                        set.len()
                    ),
                });
            }
        }

        if let Some(not_schema) = not {
            if validate_schema(&format!("{}.not", path), not_schema, definitions, value).is_ok() {
                return Err(Error::InvalidValue {
                    path: format!("{}.not", path),
                    value: value.clone(),
                    details: "value validated `not` schemas (but must not)".to_string(),
                });
            }
        }

        let if_schema_value = if_schema.as_ref().map(|if_schema| {
            validate_schema(&format!("{}.if", path), if_schema, definitions, value).is_ok()
        });

        match (if_schema_value, then_schema, else_schema) {
            (Some(_), None, None) => Err(Error::InvalidSchema {
                path: path.to_string(),
                details: "an `if` schema must have a `then` or `else`".to_string(),
            }),
            (Some(true), Some(then_schema), _) => {
                validate_schema(&format!("{}.then", path), then_schema, definitions, value)
            }
            (Some(false), _, Some(else_schema)) => {
                validate_schema(&format!("{}.else", path), else_schema, definitions, value)
            }

            (None, Some(_), None) => Err(Error::InvalidSchema {
                path: path.to_string(),
                details: "cannot have a `then` schema without an `if` schema".to_string(),
            }),
            (None, None, Some(_)) => Err(Error::InvalidSchema {
                path: path.to_string(),
                details: "cannot have an `else` schema without an `if` schema".to_string(),
            }),
            (None, Some(_), Some(_)) => Err(Error::InvalidSchema {
                path: path.to_string(),
                details: "cannot have `then` and `else` schemas without an `if` schema".to_string(),
            }),

            _ => Ok(()),
        }?;
    }

    if let Some(NumberValidation {
        multiple_of,
        maximum,
        exclusive_maximum,
        minimum,
        exclusive_minimum,
    }) = number.as_ref().map(Box::as_ref)
    {
        let n = value.as_f64().ok_or_else(|| Error::InvalidValue {
            path: path.to_string(),
            value: value.clone(),
            details: "expected a number".to_string(),
        })?;

        if let Some(multiple_of) = multiple_of {
            let div = n / multiple_of;
            if div - div.round() > f64::EPSILON {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("the value {} is not a multiple of {}", n, multiple_of),
                });
            }
        }

        if let Some(maximum) = maximum {
            if n >= *maximum {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("the value {} >= the maximum {}", n, maximum),
                });
            }
        }
        if let Some(exclusive_maximum) = exclusive_maximum {
            if n > *exclusive_maximum {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!(
                        "the value {} > the exclusive maximum {}",
                        n, exclusive_maximum
                    ),
                });
            }
        }
        if let Some(minimum) = minimum {
            if n <= *minimum {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("the value {} <= the minimum {}", n, minimum),
                });
            }
        }
        if let Some(exclusive_minimum) = exclusive_minimum {
            if n < *exclusive_minimum {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!(
                        "the value {} < the exclusive minimum {}",
                        n, exclusive_minimum
                    ),
                });
            }
        }
    }

    if let Some(StringValidation {
        max_length,
        min_length,
        pattern,
    }) = string.as_ref().map(Box::as_ref)
    {
        let s = value.as_str().ok_or_else(|| Error::InvalidValue {
            path: path.to_string(),
            value: value.clone(),
            details: "expected a string".to_string(),
        })?;

        if let Some(max_length) = max_length {
            if s.len() > *max_length as usize {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("The string is longer than {} characters", max_length),
                });
            }
        }
        if let Some(min_length) = min_length {
            if s.len() < *min_length as usize {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("The string is shorter than {} characters", min_length),
                });
            }
        }
        if let Some(pattern) = pattern {
            // ECMA 262 requires the '/' to be escaped whereas Regex does not
            // allow it. We convert sequences of '\/' into '/'.
            let prep = Regex::new(r#"((^|[^\\])(\\\\)*)\\/"#).unwrap();
            let pattern = prep.replace_all(pattern, "$1/");
            let regex = Regex::new(&pattern).map_err(|_| Error::InvalidSchema {
                path: path.to_string(),
                details: format!("{} is not a valid regex", pattern),
            })?;
            if !regex.is_match(s) {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("{} does not match tha pattern {}", s, pattern),
                });
            }
        }
    }

    if let Some(ArrayValidation {
        items,
        additional_items,
        max_items,
        min_items,
        unique_items,
        contains,
    }) = array.as_ref().map(Box::as_ref)
    {
        let arr = value.as_array().ok_or_else(|| Error::InvalidValue {
            path: path.to_string(),
            value: value.clone(),
            details: "expected an array".to_string(),
        })?;

        let arr_count = arr.len();

        if let Some(max_items) = max_items {
            if arr_count > *max_items as usize {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!(
                        "{} items is greater that the maximum of {}",
                        arr_count, max_items
                    ),
                });
            }
        }
        if let Some(min_items) = min_items {
            if arr_count < *min_items as usize {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!(
                        "{} items is less that the minimum of {}",
                        arr_count, min_items
                    ),
                });
            }
        }

        if let Some(true) = unique_items {
            for i in 0..arr_count {
                for j in 0..arr_count {
                    if i == j {
                        continue;
                    }

                    if arr[i] == arr[j] {
                        return Err(Error::InvalidValue {
                            path: path.to_string(),
                            value: value.clone(),
                            details: format!(
                                "items should be unique, but items at [{}] and [{}] are the same",
                                i, j,
                            ),
                        });
                    }
                }
            }
        }

        match items {
            Some(SingleOrVec::Single(item_schema)) => {
                arr.iter().enumerate().try_for_each(|(i, item_value)| {
                    let item_path = format!("{}[{}]", path, i);
                    validate_schema(&item_path, item_schema, definitions, item_value)
                })?;
            }
            Some(SingleOrVec::Vec(item_schemas)) => {
                arr.iter().enumerate().zip(item_schemas).try_for_each(
                    |((i, item_value), item_schema)| {
                        let item_path = format!("{}[{}]", path, i);
                        validate_schema(&item_path, item_schema, definitions, item_value)
                    },
                )?;

                if let Some(additional_schema) = additional_items {
                    arr.iter()
                        .enumerate()
                        .skip(item_schemas.len())
                        .try_for_each(|(i, item_value)| {
                            let item_path = format!("{}[{}]", path, i);
                            validate_schema(&item_path, additional_schema, definitions, item_value)
                        })?;
                }
            }
            None => (),
        }

        if let Some(contains_schema) = contains {
            if !arr.iter().enumerate().any(|(i, item_value)| {
                let item_path = format!("{}[{}]", path, i);
                validate_schema(&item_path, contains_schema, definitions, item_value).is_ok()
            }) {
                return Err(Error::InvalidValue {
                    path: format!("{}.contains", path),
                    value: value.clone(),
                    details: "array does not contain the required item".to_string(),
                });
            }
        }
    }

    if let Some(ObjectValidation {
        max_properties,
        min_properties,
        required,
        properties,
        pattern_properties,
        additional_properties,
        property_names,
    }) = object.as_ref().map(Box::as_ref)
    {
        let map = value.as_object().ok_or_else(|| Error::InvalidValue {
            path: path.to_string(),
            value: value.clone(),
            details: "expected an object".to_string(),
        })?;

        let map_count = map.iter().count();

        if let Some(max_properties) = max_properties {
            if map_count > *max_properties as usize {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!(
                        "{} properties is greater that the maximum of {}",
                        map_count, max_properties
                    ),
                });
            }
        }
        if let Some(min_properties) = min_properties {
            if map_count < *min_properties as usize {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!(
                        "{} properties is less that the minimum of {}",
                        map_count, min_properties
                    ),
                });
            }
        }

        for prop in required {
            if !map.contains_key(prop) {
                return Err(Error::InvalidValue {
                    path: path.to_string(),
                    value: value.clone(),
                    details: format!("the property {} is required but absent", prop),
                });
            }
        }

        for (prop_name, prop_value) in map {
            let prop_path = format!("{}.{}", path, prop_name);
            let mut seen = false;

            if let Some(prop_schema) = properties.get(prop_name) {
                validate_schema(&prop_path, prop_schema, definitions, prop_value)?;
                seen = true;
            }

            for (pat, pat_schema) in pattern_properties {
                if Regex::new(pat).unwrap().is_match(prop_name) {
                    validate_schema(&prop_path, pat_schema, definitions, prop_value)?;
                    seen = true;
                }
            }

            if let (false, Some(additional_schema)) = (seen, additional_properties) {
                validate_schema(&prop_path, additional_schema, definitions, prop_value)?;
            }

            if let Some(property_names_schema) = property_names {
                validate_schema(
                    &prop_path,
                    property_names_schema,
                    definitions,
                    &Value::String(prop_name.clone()),
                )?;
            }
        }
    }

    if let Some(reference) = reference {
        let idx = reference.rfind('/').ok_or_else(|| Error::InvalidSchema {
            path: path.to_string(),
            details: format!("invalid reference: {}", reference),
        })?;
        let ref_name = &reference[idx + 1..];

        let ref_schema = definitions
            .get(ref_name)
            .ok_or_else(|| Error::InvalidSchema {
                path: path.to_string(),
                details: format!("invalid reference: {}", reference),
            })?;

        validate_schema(reference, ref_schema, definitions, value)?;
    }

    Ok(())
}

fn is_valid_instance_type(instance_type: &InstanceType, value: &Value) -> bool {
    match instance_type {
        InstanceType::Null => value.is_null(),
        InstanceType::Boolean => value.is_boolean(),
        InstanceType::Object => value.is_object(),
        InstanceType::Array => value.is_array(),
        InstanceType::Number => value.is_number(),
        InstanceType::String => value.is_string(),
        InstanceType::Integer => value.is_u64() || value.is_i64(),
    }
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::Serialize;

    use crate::validate_with_output;

    #[derive(Serialize, JsonSchema)]
    #[schemars(tag = "broken")]
    enum UnmatchedEnum {
        Value,
    }

    #[derive(Serialize, JsonSchema)]
    enum MatchedEnum {
        Value,
    }

    #[test]
    fn test_matched_enum() {
        let item = MatchedEnum::Value;
        validate_with_output(&item).unwrap();
    }

    #[test]
    fn test_unmatched_enum() {
        let item = UnmatchedEnum::Value;
        match validate_with_output(&item) {
            Ok(()) => panic!("expected failure"),
            Err(msg) => expectorate::assert_contents("tests/test_unmatched_enum", &msg),
        }
    }

    #[test]
    fn test_slashes() {
        struct AmericanDate {
            year: u32,
            month: u32,
            day: u32,
        }

        impl Serialize for AmericanDate {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&format!("{}/{}/{}", self.day, self.month, self.year))
            }
        }

        impl JsonSchema for AmericanDate {
            fn schema_name() -> String {
                "AmericanDate".to_string()
            }

            fn json_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
                schemars::schema::SchemaObject {
                    string: Some(
                        schemars::schema::StringValidation {
                            pattern: Some(r#"^[0-9]{1,2}\/[0-9]{1,2}\/[0-9]{4}$"#.to_string()),
                            ..Default::default()
                        }
                        .into(),
                    ),
                    ..Default::default()
                }
                .into()
            }
        }

        let item = AmericanDate {
            year: 2017,
            month: 8,
            day: 9,
        };

        validate_with_output(&item).unwrap()
    }
}
