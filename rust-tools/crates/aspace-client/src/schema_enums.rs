use serde_json::Value;
use std::collections::{HashSet, VecDeque};

use crate::error::{ASpaceError, Result};

/// Determines whether a JSON value is truthy under Ruby semantics.
///
/// In Ruby, only `nil` and `false` evaluate to false; all other values
/// (including empty string `""`, zero `0`, empty array `[]`, and empty hash `{}`)
/// are truthy.
#[inline]
pub fn is_ruby_truthy(val: &Value) -> bool {
    !val.is_null() && !matches!(val, Value::Bool(false))
}

/// Derives dynamic enum field names from ArchivesSpace schema definitions.
///
/// Implements the Ruby breadth-first queue algorithm from `IndexerCommon#enum_fields`
/// (`indexer/app/lib/indexer_common.rb:341-364`):
/// 1. Verifies top-level `/schemas` response is a JSON Object mapping model names to model schemas.
/// 2. For each model schema, extracts `properties` (ignoring absent/null properties on empty models;
///    failing closed if `properties` is present but not an object).
/// 3. Initializes a FIFO queue of property hashes, deduplicating identical initial property hashes.
/// 4. Performs breadth-first traversal:
///    - For each Object: checks each `(k, v)`. If `v` is an Object:
///      - Checks dynamic enum presence using Ruby short-circuit semantics (`v['dynamic_enum'] || v.dig('items', 'dynamic_enum')`):
///        1) Evaluates direct `dynamic_enum` first under Ruby truthiness (`!nil && !false`). If truthy,
///           `items` is never inspected or validated, and `k` is added directly.
///        2) Only if direct `dynamic_enum` is falsy (absent, null, or false) does it inspect `items`:
///           fails closed if `items` is present but not a JSON Object or null (matching Ruby `dig`
///           TypeError on malformed non-hash `items` and keeping schema shapes strict).
///      - Then enqueues `v` for continued traversal. If `v` contains a malformed scalar `items`,
///        later traversal visits it harmlessly (it is not a JSON Object, so it emits no field,
///        and scalar queue elements match the wildcard fallback `_ => {}` harmlessly, matching
///        Ruby's `elt.is_a?(Hash)` check).
///    - For each Array: appends its elements to the queue.
/// 5. Removes all occurrences of `"items"` (matching Ruby line 362).
/// 6. Deduplicates fields in first-seen insertion order (preserving original schema order without
///    alphabetical sorting).
pub fn extract_dynamic_enum_fields_from_schemas(schemas_val: &Value) -> Result<Vec<String>> {
    let schemas_obj = schemas_val.as_object().ok_or_else(|| {
        ASpaceError::InvalidRequest("Top-level /schemas response must be a JSON object".to_string())
    })?;

    let mut initial_properties: Vec<&Value> = Vec::new();
    for (model_name, model_val) in schemas_obj {
        let model_schema = model_val.as_object().ok_or_else(|| {
            ASpaceError::InvalidRequest(format!(
                "Model schema for '{}' is not a JSON object",
                model_name
            ))
        })?;

        match model_schema.get("properties") {
            None | Some(Value::Null) => {}
            Some(props @ Value::Object(_)) => {
                if !initial_properties.contains(&props) {
                    initial_properties.push(props);
                }
            }
            Some(_) => {
                return Err(ASpaceError::InvalidRequest(format!(
                    "Model schema for '{}' has malformed 'properties' field (expected object)",
                    model_name
                )));
            }
        }
    }

    let mut queue: VecDeque<&Value> = initial_properties.into_iter().collect();
    let mut raw_fields: Vec<String> = Vec::new();

    while let Some(elt) = queue.pop_front() {
        match elt {
            Value::Object(map) => {
                for (k, v) in map {
                    if let Value::Object(v_obj) = v {
                        // Short-circuit: only inspect/validate items if direct dynamic_enum is falsy.
                        // Matches Ruby `v['dynamic_enum'] || v.dig('items', 'dynamic_enum')`.
                        let is_enum_field = if v_obj.get("dynamic_enum").is_some_and(is_ruby_truthy)
                        {
                            true
                        } else {
                            match v_obj.get("items") {
                                None | Some(Value::Null) => false,
                                Some(Value::Object(items_obj)) => {
                                    items_obj.get("dynamic_enum").is_some_and(is_ruby_truthy)
                                }
                                Some(_) => {
                                    return Err(ASpaceError::InvalidRequest(format!(
                                            "Property '{}' has malformed 'items' field (expected object)",
                                            k
                                        )));
                                }
                            }
                        };

                        if is_enum_field {
                            raw_fields.push(k.clone());
                        }
                    }
                    queue.push_back(v);
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    queue.push_back(item);
                }
            }
            _ => {}
        }
    }

    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for field in raw_fields {
        if field != "items" && seen.insert(field.clone()) {
            result.push(field);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_ruby_truthy() {
        // Only null and false are falsy in Ruby
        assert!(!is_ruby_truthy(&Value::Null));
        assert!(!is_ruby_truthy(&json!(false)));

        // Everything else is truthy
        assert!(is_ruby_truthy(&json!(true)));
        assert!(is_ruby_truthy(&json!(0)));
        assert!(is_ruby_truthy(&json!(1)));
        assert!(is_ruby_truthy(&json!(-1)));
        assert!(is_ruby_truthy(&json!("")));
        assert!(is_ruby_truthy(&json!("dynamic_enum_name")));
        assert!(is_ruby_truthy(&json!([])));
        assert!(is_ruby_truthy(&json!({})));
    }

    #[test]
    fn test_extract_dynamic_enums_basic_and_truthiness() {
        let schemas = json!({
            "model_truthiness": {
                "properties": {
                    "field_string": { "type": "string", "dynamic_enum": "valid_enum" },
                    "field_true": { "type": "string", "dynamic_enum": true },
                    "field_zero": { "type": "string", "dynamic_enum": 0 },
                    "field_empty_string": { "type": "string", "dynamic_enum": "" },
                    "field_empty_array": { "type": "string", "dynamic_enum": [] },
                    "field_empty_object": { "type": "string", "dynamic_enum": {} },
                    "field_false": { "type": "string", "dynamic_enum": false },
                    "field_null": { "type": "string", "dynamic_enum": null },
                    "field_no_enum": { "type": "string" }
                }
            }
        });

        let fields = extract_dynamic_enum_fields_from_schemas(&schemas).unwrap();
        assert_eq!(
            fields,
            vec![
                "field_string",
                "field_true",
                "field_zero",
                "field_empty_string",
                "field_empty_array",
                "field_empty_object"
            ]
        );
    }

    #[test]
    fn test_extract_dynamic_enums_array_items_and_items_exclusion() {
        let schemas = json!({
            "model_array": {
                "properties": {
                    "direct_array_enum": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "dynamic_enum": "entry_enum"
                        }
                    },
                    "items": {
                        "type": "string",
                        "dynamic_enum": "creeps_in"
                    },
                    "array_false_enum": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "dynamic_enum": false
                        }
                    },
                    "array_null_enum": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "dynamic_enum": null
                        }
                    }
                }
            }
        });

        let fields = extract_dynamic_enum_fields_from_schemas(&schemas).unwrap();
        // "items" is strictly deleted/excluded
        assert_eq!(fields, vec!["direct_array_enum"]);
    }

    #[test]
    fn test_extract_dynamic_enums_nested_and_ordering() {
        let schemas = json!({
            "first_model": {
                "properties": {
                    "zeta_field": { "type": "string", "dynamic_enum": "z_enum" },
                    "alpha_nested": {
                        "type": "object",
                        "properties": {
                            "nested_enum_field": { "type": "string", "dynamic_enum": "nested_enum" }
                        }
                    }
                }
            },
            "second_model": {
                "properties": {
                    "beta_field": { "type": "string", "dynamic_enum": "b_enum" },
                    "zeta_field": { "type": "string", "dynamic_enum": "z_enum" } // repeated
                }
            }
        });

        let fields = extract_dynamic_enum_fields_from_schemas(&schemas).unwrap();
        // Breadth-first order:
        // Level 1: zeta_field (from first), beta_field (from second), zeta_field (repeated, skipped)
        // Level 2 (nested properties): nested_enum_field
        assert_eq!(
            fields,
            vec!["zeta_field", "beta_field", "nested_enum_field"]
        );
    }

    #[test]
    fn test_extract_dynamic_enums_nested_array_items() {
        let schemas = json!({
            "model_with_array_of_objects": {
                "properties": {
                    "complex_list": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "deep_field": {
                                    "type": "string",
                                    "dynamic_enum": "deep_enum"
                                }
                            }
                        }
                    }
                }
            }
        });

        let fields = extract_dynamic_enum_fields_from_schemas(&schemas).unwrap();
        assert_eq!(fields, vec!["deep_field"]);
    }

    #[test]
    fn test_extract_dynamic_enums_empty_models() {
        let schemas = json!({
            "empty_model_no_keys": {},
            "empty_model_no_props": { "type": "object" },
            "empty_model_empty_props": { "properties": {} },
            "empty_model_null_props": { "properties": null }
        });

        let fields = extract_dynamic_enum_fields_from_schemas(&schemas).unwrap();
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_dynamic_enums_malformed_shapes_fail_closed() {
        // Top-level not an object
        assert!(extract_dynamic_enum_fields_from_schemas(&json!([])).is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!("string")).is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!(123)).is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!(null)).is_err());

        // Model schema not an object
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "bad_model": "not an object"
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "bad_model": 42
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "bad_model": []
        }))
        .is_err());

        // Properties field malformed (present, but not object or null)
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": { "properties": "not an object" }
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": { "properties": 123 }
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": { "properties": ["array", "properties"] }
        }))
        .is_err());

        // Items field malformed (present, but not object or null)
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": {
                "properties": {
                    "bad_items_array": {
                        "type": "array",
                        "items": ["not", "an", "object"]
                    }
                }
            }
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": {
                "properties": {
                    "bad_items_string": {
                        "type": "array",
                        "items": "not an object"
                    }
                }
            }
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": {
                "properties": {
                    "bad_items_number": {
                        "type": "array",
                        "items": 42
                    }
                }
            }
        }))
        .is_err());
        assert!(extract_dynamic_enum_fields_from_schemas(&json!({
            "model": {
                "properties": {
                    "bad_items_bool": {
                        "type": "array",
                        "items": true
                    }
                }
            }
        }))
        .is_err());
    }

    #[test]
    fn test_extract_dynamic_enums_short_circuit_malformed_items() {
        // dynamic_enum='valid', items='malformed' -> field accepted and no error
        let schemas_accepted = json!({
            "model_short_circuit": {
                "properties": {
                    "field_valid_enum_malformed_items": {
                        "type": "string",
                        "dynamic_enum": "valid",
                        "items": "malformed"
                    }
                }
            }
        });
        let fields = extract_dynamic_enum_fields_from_schemas(&schemas_accepted)
            .expect("dynamic_enum='valid' should short-circuit and accept field without error");
        assert_eq!(fields, vec!["field_valid_enum_malformed_items"]);

        // dynamic_enum=false, items='malformed' -> error
        let schemas_rejected = json!({
            "model_falsy_direct": {
                "properties": {
                    "field_falsy_enum_malformed_items": {
                        "type": "string",
                        "dynamic_enum": false,
                        "items": "malformed"
                    }
                }
            }
        });
        let err = extract_dynamic_enum_fields_from_schemas(&schemas_rejected)
            .expect_err("dynamic_enum=false with malformed items should fail closed with an error");
        match err {
            ASpaceError::InvalidRequest(ref msg) => {
                assert!(
                    msg.contains("Property 'field_falsy_enum_malformed_items' has malformed 'items' field (expected object)"),
                    "Unexpected error message: {}",
                    msg
                );
            }
            _ => panic!("Expected InvalidRequest error, got: {:?}", err),
        }
    }
}
