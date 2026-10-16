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
    #[serde(default)]
    pub enrich: bool,
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

const FULLRECORD_EXCLUDES: &[&str] = &[
    "created_by",
    "last_modified_by",
    "system_mtime",
    "user_mtime",
    "json",
    "types",
    "create_time",
    "date_type",
    "jsonmodel_type",
    "publish",
    "extent_type",
    "language",
    "script",
    "system_generated",
    "suppressed",
    "source",
    "rules",
    "name_order",
    "repository",
    "top_container",
    "finding_aid_filing_title",
];

fn flatten_array<'a>(value: &'a Value, output: &mut Vec<&'a Value>) {
    match value {
        Value::Array(values) => {
            for item in values {
                flatten_array(item, output);
            }
        }
        _ => output.push(value),
    }
}

fn extract_string_values(value: &Value) -> (Vec<String>, Vec<String>) {
    let mut published_queue = Vec::new();
    flatten_array(value, &mut published_queue);
    let mut unpublished_queue = Vec::new();
    let mut published_strings = Vec::new();
    let mut unpublished_strings = Vec::new();

    extract_string_values_phase(
        &mut published_queue,
        &mut published_strings,
        Some(&mut unpublished_queue),
    );
    extract_string_values_phase(&mut unpublished_queue, &mut unpublished_strings, None);

    (published_strings, unpublished_strings)
}

fn extract_string_values_phase<'a>(
    queue: &mut Vec<&'a Value>,
    strings: &mut Vec<String>,
    mut unpublished_queue: Option<&mut Vec<&'a Value>>,
) {
    while let Some(node) = queue.pop() {
        let Some(object) = node.as_object() else {
            continue;
        };
        if unpublished_queue.is_some()
            && object.contains_key("publish")
            && object.get("publish") != Some(&Value::Bool(true))
        {
            unpublished_queue
                .as_deref_mut()
                .expect("unpublished queue checked")
                .push(node);
            continue;
        }

        for (key, child) in object {
            if FULLRECORD_EXCLUDES.contains(&key.as_str()) || key.ends_with("_enum_s") {
                continue;
            }
            match child {
                Value::String(value) => strings.push(value.clone()),
                Value::Object(_) => queue.push(child),
                Value::Array(_) => {
                    let mut flattened = Vec::new();
                    flatten_array(child, &mut flattened);
                    for item in flattened {
                        match item {
                            Value::String(value) => strings.push(value.clone()),
                            Value::Object(_) => queue.push(item),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn required_string<'a>(
    value: Option<&'a Value>,
    path: &str,
) -> Result<&'a str, IndexDocumentError> {
    value
        .and_then(Value::as_str)
        .ok_or_else(|| error(format!("field '{path}' must be a string")))
}

fn add_subjects(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let Some(subjects) = record.get("subjects") else {
        return Ok(());
    };
    let subjects = subjects
        .as_array()
        .ok_or_else(|| error("field 'subjects' must be an array"))?;
    let mut titles = Vec::new();
    let mut uris = Vec::new();
    for subject in subjects {
        let subject = subject
            .as_object()
            .ok_or_else(|| error("subjects entries must be objects"))?;
        if let Some(title) = subject
            .get("_resolved")
            .and_then(Value::as_object)
            .and_then(|resolved| resolved.get("title"))
            .and_then(Value::as_str)
        {
            titles.push(Value::String(title.to_string()));
        }
        uris.push(subject.get("ref").cloned().unwrap_or(Value::Null));
    }
    document.insert("subjects".to_string(), Value::Array(titles));
    document.insert("subject_uris".to_string(), Value::Array(uris));
    Ok(())
}

fn add_agents(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let Some(agents) = record.get("linked_agents") else {
        return Ok(());
    };
    let agents = agents
        .as_array()
        .ok_or_else(|| error("field 'linked_agents' must be an array"))?;
    let mut titles = Vec::new();
    let mut uris = Vec::new();
    let mut published_titles = Vec::new();
    let mut published_uris = Vec::new();
    let mut creators = Vec::new();
    for agent in agents {
        let agent = agent
            .as_object()
            .ok_or_else(|| error("linked_agents entries must be objects"))?;
        let resolved = agent
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("linked_agents entries must contain _resolved"))?;
        let title = required_string(
            resolved
                .get("display_name")
                .and_then(Value::as_object)
                .and_then(|display| display.get("sort_name")),
            "linked_agents._resolved.display_name.sort_name",
        )?;
        let uri = agent.get("ref").cloned().unwrap_or(Value::Null);
        titles.push(Value::String(title.to_string()));
        uris.push(uri.clone());
        if resolved.get("publish") == Some(&Value::Bool(true)) {
            published_titles.push(Value::String(title.to_string()));
            published_uris.push(uri);
        }
        if agent.get("role").and_then(Value::as_str) == Some("creator") {
            creators.push(Value::String(title.to_string()));
        }
    }
    document.insert("agents".to_string(), Value::Array(titles));
    document.insert("agent_uris".to_string(), Value::Array(uris));
    document.insert(
        "published_agents".to_string(),
        Value::Array(published_titles),
    );
    document.insert(
        "published_agent_uris".to_string(),
        Value::Array(published_uris),
    );
    if !creators.is_empty() {
        document.insert("creators".to_string(), Value::Array(creators));
    }
    Ok(())
}

fn leading_year(value: Option<&Value>) -> Option<(u32, usize)> {
    let value = value?.as_str()?;
    let digits: String = value
        .chars()
        .take(4)
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some((digits.parse().ok()?, digits.len()))
    }
}

fn add_dates(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let Some(dates) = record.get("dates") else {
        return Ok(());
    };
    let dates = dates
        .as_array()
        .ok_or_else(|| error("field 'dates' must be an array"))?;
    let mut years = Vec::new();
    for date in dates {
        let date = date
            .as_object()
            .ok_or_else(|| error("dates entries must be objects"))?;
        if let Some((begin, begin_width)) = leading_year(date.get("begin")) {
            let (end, _) = leading_year(date.get("end")).unwrap_or((begin, begin_width));
            if end >= begin && end - begin <= 10_000 {
                let width = begin_width;
                years.extend((begin..=end).map(|year| format!("{year:0width$}")));
            }
        }
    }
    if !years.is_empty() {
        years.sort();
        years.dedup();
        let first = format!("{:0>4}", years.first().expect("years is not empty"));
        let last = format!("{:0>4}", years.last().expect("years is not empty"));
        document.insert(
            "years".to_string(),
            Value::Array(years.into_iter().map(Value::String).collect()),
        );
        document.insert(
            "year_sort".to_string(),
            Value::String(format!("{first}{last}")),
        );
    }

    let mut display_dates: Vec<&Value> = dates
        .iter()
        .filter(|date| date.get("date_type").and_then(Value::as_str) == Some("inclusive"))
        .collect();
    if display_dates.is_empty() {
        display_dates.extend(dates.iter());
    }
    let mut output = Vec::new();
    for date in display_dates {
        let date = date.as_object().expect("dates entries validated above");
        if let Some(expression) = date.get("expression").and_then(Value::as_str) {
            output.push(Value::String(expression.to_string()));
        } else if date.get("date_type").and_then(Value::as_str) == Some("single") {
            output.push(date.get("begin").cloned().unwrap_or(Value::Null));
        } else if date.get("date_type").is_some_and(|value| !value.is_null()) {
            let begin = date.get("begin").and_then(Value::as_str).unwrap_or("");
            let end = date.get("end").and_then(Value::as_str).unwrap_or("");
            output.push(Value::String(format!("{begin} - {end}")));
        }
    }
    document.insert("dates".to_string(), Value::Array(output));
    Ok(())
}

fn add_summary(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let Some(notes) = record.get("notes").and_then(Value::as_array) else {
        return Ok(());
    };
    if let Some(abstract_note) = notes
        .iter()
        .find(|note| note.get("type").and_then(Value::as_str) == Some("abstract"))
    {
        let content = abstract_note
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| error("abstract note content must be an array"))?;
        let strings = content
            .iter()
            .map(|value| required_string(Some(value), "notes.content").map(str::to_string))
            .collect::<Result<Vec<_>, _>>()?;
        document.insert("summary".to_string(), Value::String(strings.join("\n")));
    } else if let Some(scope_note) = notes
        .iter()
        .find(|note| note.get("type").and_then(Value::as_str) == Some("scopecontent"))
    {
        if let Some(subnotes) = scope_note.get("subnotes").and_then(Value::as_array) {
            let strings = subnotes
                .iter()
                .map(|note| {
                    required_string(note.get("content"), "notes.subnotes.content")
                        .map(str::to_string)
                })
                .collect::<Result<Vec<_>, _>>()?;
            document.insert("summary".to_string(), Value::String(strings.join("\n")));
        }
    }
    Ok(())
}

fn wrapped_values(value: Option<&Value>) -> Vec<&Value> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values.iter().collect(),
        Some(value) => vec![value],
    }
}

fn ruby_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(value) => value.to_string(),
    }
}

fn push_unique(values: &mut Vec<Value>, value: Value) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn joined_identifier(record: &Map<String, Value>, separator: &str) -> String {
    (0..4)
        .filter_map(|index| record.get(&format!("id_{index}")))
        .filter(|value| !value.is_null())
        .map(|value| ruby_string(Some(value)))
        .collect::<Vec<_>>()
        .join(separator)
}

fn add_external_ids(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let mut values = Vec::new();
    for external_id in wrapped_values(record.get("external_ids")) {
        let external_id = external_id
            .as_object()
            .ok_or_else(|| error("external_ids entries must be objects"))?;
        values.push(
            external_id
                .get("external_id")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    document.insert("external_id".to_string(), Value::Array(values));
    Ok(())
}

fn add_classifications(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let Some(classifications) = record.get("classifications") else {
        return Ok(());
    };
    let classifications = classifications
        .as_array()
        .ok_or_else(|| error("field 'classifications' must be an array"))?;
    if classifications.is_empty() {
        return Ok(());
    }

    let mut paths = Vec::new();
    let mut uris = Vec::new();
    for classification in classifications {
        let classification = classification
            .as_object()
            .ok_or_else(|| error("classifications entries must be objects"))?;
        let path = classification
            .get("_resolved")
            .and_then(Value::as_object)
            .and_then(|resolved| resolved.get("path_from_root"))
            .ok_or_else(|| error("classifications entries must resolve path_from_root"))?;
        paths.push(Value::String(serde_json::to_string(path).map_err(
            |err| error(format!("failed to serialize classification path: {err}")),
        )?));
        uris.push(classification.get("ref").cloned().unwrap_or(Value::Null));
    }
    document.insert("classification_paths".to_string(), Value::Array(paths));
    document.insert("classification_uris".to_string(), Value::Array(uris));
    Ok(())
}

fn add_instance_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let instances = wrapped_values(record.get("instances"));
    let mut digital_objects = Vec::new();
    let mut top_containers = Vec::new();
    let mut child_containers = Vec::new();
    let mut grandchild_containers = Vec::new();

    for instance in instances {
        let instance = instance
            .as_object()
            .ok_or_else(|| error("instances entries must be objects"))?;
        if let Some(digital_object) = instance.get("digital_object").and_then(Value::as_object) {
            if let Some(reference) = digital_object.get("ref") {
                match reference {
                    Value::Array(values) => {
                        for value in values {
                            push_unique(&mut digital_objects, value.clone());
                        }
                    }
                    value => push_unique(&mut digital_objects, value.clone()),
                }
            }
        }

        let Some(sub_container) = instance.get("sub_container").and_then(Value::as_object) else {
            continue;
        };
        let Some(top_container) = sub_container
            .get("top_container")
            .and_then(Value::as_object)
        else {
            continue;
        };
        top_containers.push(top_container.get("ref").cloned().unwrap_or(Value::Null));
        if sub_container
            .get("type_2")
            .is_some_and(|value| !value.is_null())
        {
            child_containers.push(Value::String(format!(
                "{} {} {}",
                ruby_string(sub_container.get("type_2")),
                ruby_string(sub_container.get("indicator_2")),
                ruby_string(sub_container.get("barcode_2"))
            )));
        }
        if sub_container
            .get("type_3")
            .is_some_and(|value| !value.is_null())
        {
            grandchild_containers.push(Value::String(format!(
                "{} {}",
                ruby_string(sub_container.get("type_3")),
                ruby_string(sub_container.get("indicator_3"))
            )));
        }
    }

    if !digital_objects.is_empty() {
        document.insert(
            "digital_object_uris".to_string(),
            Value::Array(digital_objects),
        );
    }
    if !top_containers.is_empty() {
        document.insert(
            "top_container_uri_u_sstr".to_string(),
            Value::Array(top_containers),
        );
    }
    if !child_containers.is_empty() {
        document.insert(
            "child_container_u_sstr".to_string(),
            Value::Array(child_containers),
        );
    }
    if !grandchild_containers.is_empty() {
        document.insert(
            "grand_child_container_u_sstr".to_string(),
            Value::Array(grandchild_containers),
        );
    }
    Ok(())
}

fn add_ancestors(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let ancestors = wrapped_values(record.get("ancestors"))
        .into_iter()
        .map(|ancestor| {
            ancestor
                .as_object()
                .and_then(|ancestor| ancestor.get("ref"))
                .cloned()
                .ok_or_else(|| error("ancestors entries must contain ref"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    document.insert("ancestors".to_string(), Value::Array(ancestors));
    Ok(())
}

fn add_rights_statement_agents(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let mut uris = Vec::new();
    for statement in wrapped_values(record.get("rights_statements")) {
        let statement = statement
            .as_object()
            .ok_or_else(|| error("rights_statements entries must be objects"))?;
        for agent in wrapped_values(statement.get("linked_agents")) {
            let agent = agent
                .as_object()
                .ok_or_else(|| error("rights statement linked_agents entries must be objects"))?;
            uris.push(agent.get("ref").cloned().unwrap_or(Value::Null));
        }
    }
    if !uris.is_empty() {
        document.insert(
            "rights_statement_agent_uris".to_string(),
            Value::Array(uris),
        );
    }
    Ok(())
}

fn add_languages(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let mut languages = Vec::new();
    if let Some(materials) = record.get("lang_materials").and_then(Value::as_array) {
        for material in materials {
            let material = material
                .as_object()
                .ok_or_else(|| error("lang_materials entries must be objects"))?;
            if let Some(language_and_script) = material
                .get("language_and_script")
                .and_then(Value::as_object)
            {
                push_unique(
                    &mut languages,
                    language_and_script
                        .get("language")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
        }
    }
    document.insert("langcode".to_string(), Value::Array(languages));
    Ok(())
}

fn add_resource_and_archival_object_fields(
    document: &mut Map<String, Value>,
    record_type: &str,
    record: &Map<String, Value>,
) -> Result<bool, IndexDocumentError> {
    if !matches!(record_type, "resource" | "archival_object") {
        return Ok(false);
    }

    if record_type == "archival_object" {
        if let Some(resource) = record.get("resource").and_then(Value::as_object) {
            document.insert(
                "resource".to_string(),
                resource.get("ref").cloned().unwrap_or(Value::Null),
            );
        }
        for (document_field, record_field) in [
            ("title", "display_string"),
            ("identifier", "component_id"),
            ("component_id", "component_id"),
            ("ref_id", "ref_id"),
            ("slug", "slug"),
            ("is_slug_auto", "is_slug_auto"),
        ] {
            document.insert(
                document_field.to_string(),
                record.get(record_field).cloned().unwrap_or(Value::Null),
            );
        }
    } else {
        for field in [
            "finding_aid_title",
            "finding_aid_filing_title",
            "resource_type",
            "level",
            "restrictions",
            "ead_id",
            "finding_aid_status",
            "slug",
            "is_slug_auto",
        ] {
            document.insert(
                field.to_string(),
                record.get(field).cloned().unwrap_or(Value::Null),
            );
        }
        document.insert(
            "identifier".to_string(),
            Value::String(joined_identifier(record, "-")),
        );

        let related_accessions = record
            .get("related_accessions")
            .map(|value| {
                value
                    .as_array()
                    .ok_or_else(|| error("field 'related_accessions' must be an array"))
            })
            .transpose()?
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut related_uris = Vec::new();
        for accession in related_accessions {
            if let Some(reference) = accession.get("ref") {
                if !reference.is_null() {
                    push_unique(&mut related_uris, reference.clone());
                }
            }
        }
        document.insert(
            "related_accession_uris".to_string(),
            Value::Array(related_uris),
        );

        if let Some(collection_management) = record
            .get("collection_management")
            .and_then(Value::as_object)
        {
            for field in ["processing_priority", "processors"] {
                document.insert(
                    field.to_string(),
                    collection_management
                        .get(field)
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
        }
        add_classifications(document, record)?;
    }

    add_external_ids(document, record)?;
    add_instance_fields(document, record)?;
    let four_part_id = joined_identifier(record, " ");
    if !four_part_id.is_empty() {
        document.insert("four_part_id".to_string(), Value::String(four_part_id));
    }
    add_ancestors(document, record)?;
    add_rights_statement_agents(document, record)?;
    add_languages(document, record)?;
    Ok(true)
}

fn add_enriched_fields(
    document: &mut Map<String, Value>,
    record_type: &str,
    record: &Map<String, Value>,
    full_record: &Value,
) -> Result<(), IndexDocumentError> {
    add_subjects(document, record)?;
    add_agents(document, record)?;
    for field in [
        "created_by",
        "last_modified_by",
        "user_mtime",
        "system_mtime",
        "create_time",
    ] {
        if let Some(value) = record.get(field) {
            document.insert(field.to_string(), value.clone());
        }
    }
    if let Some(notes) = record.get("notes") {
        if notes.is_string() {
            document.insert("notes_published".to_string(), notes.clone());
            document.insert("notes".to_string(), notes.clone());
        } else {
            let (published, unpublished) = extract_string_values(notes);
            document.insert(
                "notes_published".to_string(),
                Value::Array(published.into_iter().map(Value::String).collect()),
            );
            document.insert(
                "notes".to_string(),
                Value::Array(unpublished.into_iter().map(Value::String).collect()),
            );
        }
    }
    add_dates(document, record)?;
    if let Some(level) = record.get("level") {
        let level = if level.as_str() == Some("otherlevel") {
            record.get("other_level").cloned().unwrap_or(Value::Null)
        } else {
            level.clone()
        };
        document.insert("level".to_string(), level);
    }
    add_summary(document, record)?;
    if let Some(extents) = record.get("extents") {
        let extents = extents
            .as_array()
            .ok_or_else(|| error("field 'extents' must be an array"))?;
        let whole: Vec<&Value> = extents
            .iter()
            .filter(|extent| extent.get("portion").and_then(Value::as_str) == Some("whole"))
            .collect();
        let display = if whole.is_empty() {
            extents.iter().collect::<Vec<_>>()
        } else {
            whole
        };
        let values = display
            .into_iter()
            .map(|extent| {
                let number = extent.get("number").and_then(Value::as_str).unwrap_or("");
                let extent_type = extent
                    .get("extent_type")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Value::String(format!("{number} --- {extent_type}"))
            })
            .collect();
        document.insert("extents".to_string(), Value::Array(values));
    }
    let (published, unpublished) = extract_string_values(full_record);
    document.insert(
        "fullrecord_published".to_string(),
        Value::Array(published.into_iter().map(Value::String).collect()),
    );
    document.insert(
        "fullrecord".to_string(),
        Value::Array(unpublished.into_iter().map(Value::String).collect()),
    );
    let mut rust_fields = vec![json_string("common_metadata"), json_string("fullrecord")];
    if add_resource_and_archival_object_fields(document, record_type, record)? {
        rust_fields.push(json_string("resource_archival_object_metadata"));
    }
    document.insert(
        "_archivalist_rust_fields".to_string(),
        Value::Array(rust_fields),
    );
    Ok(())
}

fn json_string(value: &str) -> Value {
    Value::String(value.to_string())
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
        Value::Array(vec![Value::String(input.record_type.clone())]),
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

    if input.enrich {
        add_enriched_fields(&mut document, &input.record_type, record, &input.record)?;
    }

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
            enrich: false,
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
    fn builds_staff_index_enrichment_fields() {
        let mut enriched = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Collection title",
            "publish": true,
            "created_by": "admin",
            "level": "otherlevel",
            "other_level": "subfonds",
            "subjects": [{
                "ref": "/subjects/1",
                "_resolved": {"title": "Irish archives"}
            }],
            "linked_agents": [{
                "ref": "/agents/people/1",
                "role": "creator",
                "_resolved": {
                    "publish": true,
                    "display_name": {"sort_name": "Sánchez, Luis"}
                }
            }],
            "dates": [{
                "date_type": "inclusive",
                "begin": "1974",
                "end": "1976",
                "expression": "1974–1976"
            }],
            "extents": [
                {"portion": "part", "number": "1", "extent_type": "boxes"},
                {"portion": "whole", "number": "3", "extent_type": "linear_feet"}
            ],
            "notes": [
                {"type": "abstract", "publish": true, "content": ["Public abstract"]},
                {"type": "scopecontent", "publish": false, "content": ["Private note"]}
            ],
            "top_container": {"internal_note": "Must be excluded"}
        }));
        enriched.enrich = true;

        let document = build_index_document(enriched).unwrap();
        assert_eq!(document["subjects"], json!(["Irish archives"]));
        assert_eq!(document["subject_uris"], json!(["/subjects/1"]));
        assert_eq!(document["agents"], json!(["Sánchez, Luis"]));
        assert_eq!(document["creators"], json!(["Sánchez, Luis"]));
        assert_eq!(document["published_agents"], json!(["Sánchez, Luis"]));
        assert_eq!(document["years"], json!(["1974", "1975", "1976"]));
        assert_eq!(document["year_sort"], "19741976");
        assert_eq!(document["dates"], json!(["1974–1976"]));
        assert_eq!(document["level"], "otherlevel");
        assert_eq!(document["summary"], "Public abstract");
        assert_eq!(document["extents"], json!(["3 --- linear_feet"]));
        assert_eq!(
            document["notes_published"],
            json!(["abstract", "Public abstract"])
        );
        assert_eq!(document["notes"], json!(["scopecontent", "Private note"]));
        assert_eq!(document["created_by"], "admin");
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "resource_archival_object_metadata"
            ])
        );

        let published = document["fullrecord_published"].as_array().unwrap();
        let unpublished = document["fullrecord"].as_array().unwrap();
        assert!(published.contains(&json!("Public abstract")));
        assert!(!published.contains(&json!("Private note")));
        assert!(unpublished.contains(&json!("Private note")));
        assert!(!published.contains(&json!("Must be excluded")));
        assert!(!published.contains(&json!("linear_feet")));
    }

    #[test]
    fn builds_resource_specific_staff_index_fields() {
        let mut enriched = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Collection title",
            "publish": true,
            "finding_aid_title": "Collection guide",
            "finding_aid_filing_title": "Guide, Collection",
            "id_0": "MS",
            "id_1": "7",
            "resource_type": "papers",
            "level": "collection",
            "restrictions": true,
            "ead_id": "ms-7",
            "finding_aid_status": "completed",
            "related_accessions": [
                {"ref": "/repositories/2/accessions/1"},
                {"ref": "/repositories/2/accessions/1"}
            ],
            "slug": "collection-title",
            "is_slug_auto": false,
            "collection_management": {
                "processing_priority": "high",
                "processors": "A. Archivist"
            },
            "external_ids": [{"external_id": "legacy-7"}],
            "classifications": [{
                "ref": "/classifications/1",
                "_resolved": {"path_from_root": [{"identifier": "COL"}]}
            }],
            "instances": [
                {"digital_object": {"ref": "/repositories/2/digital_objects/3"}},
                {"sub_container": {
                    "top_container": {"ref": "/repositories/2/top_containers/4"},
                    "type_2": "folder",
                    "indicator_2": "2",
                    "barcode_2": "B2",
                    "type_3": "item",
                    "indicator_3": "5"
                }}
            ],
            "ancestors": [{"ref": "/repositories/2/resources/1"}],
            "rights_statements": [{
                "linked_agents": [{"ref": "/agents/people/9"}]
            }],
            "lang_materials": [
                {"language_and_script": {"language": "eng"}},
                {"language_and_script": {"language": "eng"}}
            ]
        }));
        enriched.enrich = true;

        let document = build_index_document(enriched).unwrap();
        assert_eq!(document["finding_aid_title"], "Collection guide");
        assert_eq!(document["identifier"], "MS-7");
        assert_eq!(document["four_part_id"], "MS 7");
        assert_eq!(
            document["related_accession_uris"],
            json!(["/repositories/2/accessions/1"])
        );
        assert_eq!(document["external_id"], json!(["legacy-7"]));
        assert_eq!(
            document["classification_uris"],
            json!(["/classifications/1"])
        );
        assert_eq!(
            document["digital_object_uris"],
            json!(["/repositories/2/digital_objects/3"])
        );
        assert_eq!(
            document["top_container_uri_u_sstr"],
            json!(["/repositories/2/top_containers/4"])
        );
        assert_eq!(document["child_container_u_sstr"], json!(["folder 2 B2"]));
        assert_eq!(document["grand_child_container_u_sstr"], json!(["item 5"]));
        assert_eq!(
            document["ancestors"],
            json!(["/repositories/2/resources/1"])
        );
        assert_eq!(
            document["rights_statement_agent_uris"],
            json!(["/agents/people/9"])
        );
        assert_eq!(document["langcode"], json!(["eng"]));
        assert_eq!(document["processing_priority"], "high");
    }

    #[test]
    fn builds_archival_object_specific_staff_index_fields() {
        let mut enriched = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/8".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/8",
                "title": "Original title",
                "display_string": "Series I: Correspondence",
                "publish": true,
                "resource": {"ref": "/repositories/2/resources/7"},
                "component_id": "I",
                "ref_id": "ref8",
                "slug": "series-i-correspondence",
                "is_slug_auto": true,
                "external_ids": [],
                "ancestors": [{"ref": "/repositories/2/resources/7"}],
                "lang_materials": []
            }),
            enrich: true,
        };
        enriched.enrich = true;

        let document = build_index_document(enriched).unwrap();
        assert_eq!(document["title"], "Series I: Correspondence");
        assert_eq!(document["resource"], "/repositories/2/resources/7");
        assert_eq!(document["identifier"], "I");
        assert_eq!(document["component_id"], "I");
        assert_eq!(document["ref_id"], "ref8");
        assert_eq!(document["slug"], "series-i-correspondence");
        assert_eq!(document["is_slug_auto"], true);
        assert_eq!(document["external_id"], json!([]));
        assert_eq!(
            document["ancestors"],
            json!(["/repositories/2/resources/7"])
        );
        assert_eq!(document["langcode"], json!([]));
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
    fn enrichment_matches_string_notes_and_short_year_ranges() {
        let mut enriched = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Edge shapes",
            "notes": "Profile note",
            "dates": [{"date_type": "inclusive", "begin": "9", "end": "11"}]
        }));
        enriched.enrich = true;
        let document = build_index_document(enriched).unwrap();
        assert_eq!(document["notes_published"], "Profile note");
        assert_eq!(document["notes"], "Profile note");
        assert_eq!(document["years"], json!(["10", "11", "9"]));
        assert_eq!(document["year_sort"], "00100009");
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
