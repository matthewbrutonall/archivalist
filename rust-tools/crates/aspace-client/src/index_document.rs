use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub const MAX_INDEX_DOCUMENT_BATCH: usize = 1_000;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct IndexDocumentInput {
    pub uri: String,
    pub record_type: String,
    pub repository: Value,
    pub record: Value,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct IndexDocumentBatchRequest {
    pub records: Vec<IndexDocumentInput>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct IndexDocumentBatchResponse {
    pub documents: Vec<Value>,
    pub served_by: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct IndexDocumentError(String);

impl fmt::Display for IndexDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for IndexDocumentError {}

fn error(message: impl Into<String>) -> IndexDocumentError {
    IndexDocumentError(message.into())
}

fn optional_bool(
    record: &Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, IndexDocumentError> {
    match record.get(field) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(error(format!(
            "field '{field}' must be a boolean when present"
        ))),
    }
}

fn system_generated(record: &Map<String, Value>) -> Result<String, IndexDocumentError> {
    match record.get("system_generated") {
        None => Ok("false".to_string()),
        Some(Value::Bool(value)) => Ok(value.to_string()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Number(value)) => Ok(value.to_string()),
        Some(_) => Err(error(
            "field 'system_generated' must be a boolean, string, or number when present",
        )),
    }
}

fn repository_is_unpublished(record: &Map<String, Value>) -> bool {
    record
        .get("repository")
        .and_then(Value::as_object)
        .and_then(|repository| repository.get("_resolved"))
        .and_then(Value::as_object)
        .and_then(|resolved| resolved.get("publish"))
        == Some(&Value::Bool(false))
}

pub fn build_index_document(input: IndexDocumentInput) -> Result<Value, IndexDocumentError> {
    if input.uri.trim().is_empty() {
        return Err(error("uri must not be empty"));
    }
    if input.record_type.trim().is_empty() {
        return Err(error("record_type must not be empty"));
    }

    let record = input
        .record
        .as_object()
        .ok_or_else(|| error("record must be a JSON object"))?;
    if let Some(record_uri) = record.get("uri").and_then(Value::as_str) {
        if record_uri != input.uri {
            return Err(error(format!(
                "record uri '{record_uri}' does not match wrapper uri '{}'",
                input.uri
            )));
        }
    }

    let suppressed = optional_bool(record, "suppressed")?.unwrap_or(false);
    let has_unpublished_ancestor =
        optional_bool(record, "has_unpublished_ancestor")?.unwrap_or(false);
    let record_publish = optional_bool(record, "publish")?.unwrap_or(false);
    let publish = !suppressed
        && !repository_is_unpublished(record)
        && !has_unpublished_ancestor
        && record_publish;
    let title = record.get("title").cloned().unwrap_or(Value::Null);
    if !title.is_null() && !title.is_string() {
        return Err(error("field 'title' must be a string or null when present"));
    }

    let mut document = Map::new();
    document.insert("id".to_string(), Value::String(input.uri.clone()));
    document.insert("uri".to_string(), Value::String(input.uri));
    document.insert("title".to_string(), title);
    document.insert(
        "primary_type".to_string(),
        Value::String(input.record_type.clone()),
    );
    document.insert(
        "types".to_string(),
        Value::Array(vec![Value::String(input.record_type)]),
    );
    document.insert(
        "json".to_string(),
        Value::String(
            serde_json::to_string(&input.record)
                .map_err(|err| error(format!("failed to serialize record: {err}")))?,
        ),
    );
    document.insert("suppressed".to_string(), Value::Bool(suppressed));
    document.insert("publish".to_string(), Value::Bool(publish));
    document.insert(
        "system_generated".to_string(),
        Value::String(system_generated(record)?),
    );
    document.insert("repository".to_string(), input.repository);

    Ok(Value::Object(document))
}

pub fn build_index_document_batch(
    request: IndexDocumentBatchRequest,
) -> Result<IndexDocumentBatchResponse, IndexDocumentError> {
    if request.records.is_empty() {
        return Err(error("records must contain at least one item"));
    }
    if request.records.len() > MAX_INDEX_DOCUMENT_BATCH {
        return Err(error(format!(
            "records exceeds the maximum batch size of {MAX_INDEX_DOCUMENT_BATCH}"
        )));
    }

    let documents = request
        .records
        .into_iter()
        .map(build_index_document)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(IndexDocumentBatchResponse {
        documents,
        served_by: "archivalist-rust-index-document-builder".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(record: Value) -> IndexDocumentInput {
        IndexDocumentInput {
            uri: "/repositories/2/resources/7".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record,
        }
    }

    #[test]
    fn builds_the_common_solr_document_envelope() {
        let document = build_index_document(input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Sánchez papers",
            "publish": true,
            "suppressed": false,
            "system_generated": true
        })))
        .unwrap();

        assert_eq!(document["id"], "/repositories/2/resources/7");
        assert_eq!(document["title"], "Sánchez papers");
        assert_eq!(document["types"], json!(["resource"]));
        assert_eq!(document["publish"], true);
        assert_eq!(document["system_generated"], "true");
        assert_eq!(document["repository"], 2);
        let embedded: Value = serde_json::from_str(document["json"].as_str().unwrap()).unwrap();
        assert_eq!(embedded["title"], "Sánchez papers");
    }

    #[test]
    fn applies_every_publish_suppression_rule() {
        for extra in [
            json!({"suppressed": true}),
            json!({"has_unpublished_ancestor": true}),
            json!({"repository": {"_resolved": {"publish": false}}}),
        ] {
            let mut record = json!({
                "uri": "/repositories/2/resources/7",
                "title": "Hidden",
                "publish": true
            });
            record
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert_eq!(
                build_index_document(input(record)).unwrap()["publish"],
                false
            );
        }
    }

    #[test]
    fn rejects_invalid_or_mismatched_records() {
        assert!(build_index_document(input(json!([]))).is_err());
        assert!(build_index_document(input(json!({"uri": "/wrong"}))).is_err());
        assert!(build_index_document(input(json!({
            "uri": "/repositories/2/resources/7",
            "publish": "yes"
        })))
        .is_err());
    }

    #[test]
    fn enforces_nonempty_bounded_batches() {
        assert!(build_index_document_batch(IndexDocumentBatchRequest { records: vec![] }).is_err());
        let records = (0..=MAX_INDEX_DOCUMENT_BATCH)
            .map(|_| input(json!({"uri": "/repositories/2/resources/7"})))
            .collect();
        assert!(build_index_document_batch(IndexDocumentBatchRequest { records }).is_err());
    }
}
