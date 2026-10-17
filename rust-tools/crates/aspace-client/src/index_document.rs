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

fn related_uris(
    record: &Map<String, Value>,
    field: &str,
    compact: bool,
    unique: bool,
) -> Result<Vec<Value>, IndexDocumentError> {
    let values = match record.get(field) {
        None => &[][..],
        Some(value) => value
            .as_array()
            .ok_or_else(|| error(format!("field '{field}' must be an array")))?,
    };
    let mut uris = Vec::new();
    for value in values {
        let reference = value.get("ref").cloned().unwrap_or(Value::Null);
        if compact && reference.is_null() {
            continue;
        }
        if unique {
            push_unique(&mut uris, reference);
        } else {
            uris.push(reference);
        }
    }
    Ok(uris)
}

fn add_collection_management_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) {
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
}

fn ruby_truthy(value: Option<&Value>) -> bool {
    !matches!(value, None | Some(Value::Null) | Some(Value::Bool(false)))
}

fn add_used_within_fields(document: &mut Map<String, Value>, record: &Map<String, Value>) {
    if record.contains_key("used_within_repositories") {
        document.insert(
            "used_within_repository".to_string(),
            record
                .get("used_within_repositories")
                .cloned()
                .unwrap_or(Value::Null),
        );
        document.insert(
            "used_within_published_repository".to_string(),
            record
                .get("used_within_published_repositories")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
}

fn add_agent_subject_links(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let mut subjects = document
        .remove("subjects")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut subject_uris = document
        .remove("subject_uris")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    for (subrecord_field, link_field) in [
        ("agent_functions", "subjects"),
        ("agent_occupations", "subjects"),
        ("agent_places", "subjects"),
        ("agent_topics", "subjects"),
        ("agent_functions", "places"),
        ("agent_occupations", "places"),
        ("agent_resources", "places"),
        ("agent_topics", "places"),
    ] {
        for subrecord in wrapped_values(record.get(subrecord_field)) {
            let subrecord = subrecord
                .as_object()
                .ok_or_else(|| error(format!("{subrecord_field} entries must be objects")))?;
            let Some(links) = subrecord.get(link_field) else {
                continue;
            };
            let links = links
                .as_array()
                .ok_or_else(|| error(format!("{subrecord_field}.{link_field} must be an array")))?;
            for link in links {
                let link = link.as_object().ok_or_else(|| {
                    error(format!(
                        "{subrecord_field}.{link_field} entries must be objects"
                    ))
                })?;
                if let Some(title) = link
                    .get("_resolved")
                    .and_then(Value::as_object)
                    .and_then(|resolved| resolved.get("title"))
                    .filter(|title| !title.is_null())
                {
                    subjects.push(title.clone());
                }
                subject_uris.push(link.get("ref").cloned().unwrap_or(Value::Null));
            }
        }
    }

    document.insert("subjects".to_string(), Value::Array(subjects));
    document.insert("subject_uris".to_string(), Value::Array(subject_uris));
    Ok(())
}

fn add_subject_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let terms = record
        .get("terms")
        .and_then(Value::as_array)
        .ok_or_else(|| error("field 'terms' must be an array"))?;
    let first_term = terms
        .first()
        .and_then(Value::as_object)
        .ok_or_else(|| error("field 'terms' must contain an object"))?;
    for field in ["source", "slug", "is_slug_auto"] {
        document.insert(
            field.to_string(),
            record.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    document.insert(
        "first_term_type".to_string(),
        first_term.get("term_type").cloned().unwrap_or(Value::Null),
    );
    document.insert(
        "publish".to_string(),
        Value::Bool(
            ruby_truthy(record.get("publish"))
                && ruby_truthy(record.get("is_linked_to_published_record")),
        ),
    );
    add_used_within_fields(document, record);
    Ok(())
}

fn add_agent_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let display_name = record
        .get("display_name")
        .and_then(Value::as_object)
        .ok_or_else(|| error("field 'display_name' must be an object"))?;
    document.insert(
        "title".to_string(),
        Value::String(
            required_string(display_name.get("sort_name"), "display_name.sort_name")?.to_string(),
        ),
    );

    let names = record
        .get("names")
        .and_then(Value::as_array)
        .ok_or_else(|| error("field 'names' must be an array"))?;
    let authorized_name = names
        .iter()
        .filter_map(Value::as_object)
        .find(|name| ruby_truthy(name.get("authorized")));
    let record_identifiers = record
        .get("agent_record_identifiers")
        .and_then(Value::as_array)
        .ok_or_else(|| error("field 'agent_record_identifiers' must be an array"))?;
    let primary_identifier = record_identifiers
        .iter()
        .filter_map(Value::as_object)
        .find(|identifier| identifier.get("primary_identifier") == Some(&Value::Bool(true)));

    if let Some(identifier) = primary_identifier {
        document.insert(
            "authority_id".to_string(),
            identifier
                .get("record_identifier")
                .cloned()
                .unwrap_or(Value::Null),
        );
        document.insert(
            "source".to_string(),
            identifier.get("source").cloned().unwrap_or(Value::Null),
        );
    } else if let Some(name) = authorized_name {
        document.insert(
            "authority_id".to_string(),
            name.get("authority_id").cloned().unwrap_or(Value::Null),
        );
        document.insert(
            "source".to_string(),
            name.get("source").cloned().unwrap_or(Value::Null),
        );
    }

    let conventions = record
        .get("agent_conventions_declarations")
        .and_then(Value::as_array)
        .ok_or_else(|| error("field 'agent_conventions_declarations' must be an array"))?;
    if let Some(rule) = conventions
        .first()
        .and_then(Value::as_object)
        .and_then(|declaration| declaration.get("name_rule"))
        .filter(|rule| ruby_truthy(Some(rule)))
    {
        document.insert("rules".to_string(), rule.clone());
    } else if let Some(name) = authorized_name {
        document.insert(
            "rules".to_string(),
            name.get("rules").cloned().unwrap_or(Value::Null),
        );
    }

    for field in ["linked_agent_roles", "slug", "is_slug_auto"] {
        document.insert(
            field.to_string(),
            record.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    document.insert(
        "related_agent_uris".to_string(),
        Value::Array(related_uris(record, "related_agents", false, false)?),
    );

    let is_user = ruby_truthy(record.get("is_user"));
    document.insert("is_user".to_string(), Value::Bool(is_user));
    let types = document
        .get_mut("types")
        .and_then(Value::as_array_mut)
        .expect("types is initialized as an array");
    if is_user {
        types.push(json_string("agent_with_user"));
    }
    types.push(json_string("agent"));

    add_agent_subject_links(document, record)?;
    add_used_within_fields(document, record);
    Ok(())
}

fn add_digital_object_component_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let digital_object = record
        .get("digital_object")
        .and_then(Value::as_object)
        .ok_or_else(|| error("field 'digital_object' must be an object"))?;
    document.insert(
        "digital_object".to_string(),
        digital_object.get("ref").cloned().unwrap_or(Value::Null),
    );
    let identifier = record.get("component_id").cloned().unwrap_or(Value::Null);
    document.insert("digital_object_id".to_string(), identifier.clone());
    document.insert("identifier".to_string(), identifier);
    for (document_field, record_field) in [
        ("title", "display_string"),
        ("slug", "slug"),
        ("is_slug_auto", "is_slug_auto"),
    ] {
        document.insert(
            document_field.to_string(),
            record.get(record_field).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(())
}

fn classification_identifier_sort(identifier: &str) -> String {
    let mut digit_start = identifier.len();
    let mut digit_end = identifier.len();
    for (index, character) in identifier.char_indices() {
        if character.is_ascii_digit() {
            digit_start = index;
            digit_end = index;
            break;
        }
    }
    if digit_start < identifier.len() {
        for (offset, character) in identifier[digit_start..].char_indices() {
            if !character.is_ascii_digit() {
                digit_end = digit_start + offset;
                break;
            }
            digit_end = digit_start + offset + character.len_utf8();
        }
    }

    let letters = identifier[..digit_start].trim();
    let numbers = identifier[digit_start..digit_end].trim();
    let rest = identifier[digit_end..].trim();
    let letters = format!("{letters:<255}").replace(' ', "#");
    let numbers = format!("{numbers:0>255}");
    let rest = format!("{rest:<255}");
    format!("{letters}{numbers}{rest}")
}

fn add_classification_fields(
    document: &mut Map<String, Value>,
    record_type: &str,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let path = record.get("path_from_root").cloned().unwrap_or(Value::Null);
    document.insert(
        "classification_path".to_string(),
        Value::String(serde_json::to_string(&path).map_err(|source| {
            error(format!("failed to serialize classification path: {source}"))
        })?),
    );

    let mut agent_uris = Vec::new();
    let mut published_agent_uris = Vec::new();
    let mut agents = Vec::new();
    for creator in wrapped_values(record.get("creator")) {
        let creator = creator
            .as_object()
            .ok_or_else(|| error("field 'creator' must contain objects"))?;
        let reference = creator.get("ref").cloned().unwrap_or(Value::Null);
        agent_uris.push(reference.clone());
        let resolved = creator
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("creator entries must contain _resolved"))?;
        let sort_name = required_string(
            resolved
                .get("display_name")
                .and_then(Value::as_object)
                .and_then(|display_name| display_name.get("sort_name")),
            "creator._resolved.display_name.sort_name",
        )?;
        agents.push(Value::String(sort_name.to_string()));
        if ruby_truthy(resolved.get("publish")) && !reference.is_null() {
            published_agent_uris.push(reference);
        }
    }
    document.insert("agent_uris".to_string(), Value::Array(agent_uris));
    document.insert(
        "published_agent_uris".to_string(),
        Value::Array(published_agent_uris),
    );
    document.insert("agents".to_string(), Value::Array(agents));

    let identifier = required_string(record.get("identifier"), "identifier")?;
    document.insert(
        "identifier_sort".to_string(),
        Value::String(classification_identifier_sort(identifier)),
    );
    let repo_sort = record
        .get("repository")
        .and_then(Value::as_object)
        .and_then(|repository| repository.get("_resolved"))
        .and_then(Value::as_object)
        .and_then(|resolved| resolved.get("display_string"))
        .cloned()
        .unwrap_or(Value::Null);
    document.insert("repo_sort".to_string(), repo_sort);
    for field in [
        "has_classification_terms",
        "slug",
        "is_slug_auto",
        "identifier",
    ] {
        document.insert(
            field.to_string(),
            record.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    if record_type == "classification_term" {
        let classification = record
            .get("classification")
            .and_then(Value::as_object)
            .ok_or_else(|| error("field 'classification' must be an object"))?;
        document.insert(
            "classification".to_string(),
            classification.get("ref").cloned().unwrap_or(Value::Null),
        );
    }
    Ok(())
}

fn strip_angle_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        match after_open.find('>') {
            Some(0) => {
                output.push('<');
                rest = after_open;
            }
            Some(close) => {
                rest = &after_open[close + 1..];
            }
            None => {
                output.push_str(&rest[open..]);
                return output;
            }
        }
    }
    output.push_str(rest);
    output
}

fn clean_for_sort(value: &str) -> String {
    let without_tags = strip_angle_tags(value);
    let filtered: String = without_tags
        .chars()
        .map(|character| if character == '-' { ' ' } else { character })
        .filter(|character| {
            character.is_alphabetic()
                || character.is_ascii_digit()
                || character.is_whitespace()
                || *character == '_'
        })
        .collect();
    filtered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn add_repository_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let id = document.get("id").cloned().unwrap_or(Value::Null);
    document.insert("repository".to_string(), id);
    document.insert(
        "title".to_string(),
        record.get("repo_code").cloned().unwrap_or(Value::Null),
    );
    let title_sort = match record.get("display_string") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(value)) => Value::String(clean_for_sort(value)),
        Some(_) => return Err(error("field 'display_string' must be a string")),
    };
    document.insert("title_sort".to_string(), title_sort);
    document.insert(
        "repo_sort".to_string(),
        record.get("display_string").cloned().unwrap_or(Value::Null),
    );
    for (document_field, record_field) in [
        ("slug", "slug"),
        ("is_slug_auto", "is_slug_auto"),
        ("position_int_sort", "position"),
    ] {
        document.insert(
            document_field.to_string(),
            record.get(record_field).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(())
}

fn add_event_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let event_type = record.get("event_type").cloned().unwrap_or(Value::Null);
    document.insert("event_type".to_string(), event_type.clone());
    document.insert("title".to_string(), event_type);
    document.insert(
        "outcome".to_string(),
        record.get("outcome").cloned().unwrap_or(Value::Null),
    );

    let linked_records = record
        .get("linked_records")
        .and_then(Value::as_array)
        .ok_or_else(|| error("field 'linked_records' must be an array"))?;
    let mut linked_record_uris = Vec::new();
    let mut linked_record_titles = Vec::new();
    for linked_record in linked_records {
        let linked_record = linked_record
            .as_object()
            .ok_or_else(|| error("linked_records entries must be objects"))?;
        linked_record_uris.push(linked_record.get("ref").cloned().unwrap_or(Value::Null));
        let resolved = linked_record
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("linked_records entries must contain _resolved"))?;
        let title = match resolved.get("jsonmodel_type").and_then(Value::as_str) {
            Some("agent_person" | "agent_family" | "agent_corporate_entity" | "agent_software") => {
                let display_name = resolved
                    .get("display_name")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        error("linked_records agent entries must contain _resolved.display_name")
                    })?;
                display_name
                    .get("sort_name")
                    .cloned()
                    .unwrap_or(Value::Null)
            }
            Some("accession" | "resource" | "digital_object" | "digital_object_component") => {
                resolved.get("title").cloned().unwrap_or(Value::Null)
            }
            Some("archival_object") => resolved
                .get("display_string")
                .cloned()
                .unwrap_or(Value::Null),
            _ => Value::String("not_found".to_string()),
        };
        linked_record_titles.push(title);
    }
    document.insert(
        "linked_record_uris".to_string(),
        Value::Array(linked_record_uris),
    );
    document.insert(
        "linked_record_titles".to_string(),
        Value::Array(linked_record_titles),
    );
    Ok(())
}

fn add_location_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    if let Some(temporary) = record.get("temporary") {
        document.insert("temporary".to_string(), temporary.clone());
    }
    for field in ["building", "floor", "room", "area"] {
        document.insert(
            field.to_string(),
            record.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    if let Some(owner_repo) = record
        .get("owner_repo")
        .filter(|value| ruby_truthy(Some(value)))
    {
        let owner_repo = owner_repo
            .as_object()
            .ok_or_else(|| error("field 'owner_repo' must be an object"))?;
        document.insert(
            "owner_repo_uri_u_sstr".to_string(),
            owner_repo.get("ref").cloned().unwrap_or(Value::Null),
        );
        if let Some(repo_code) = owner_repo
            .get("_resolved")
            .and_then(Value::as_object)
            .and_then(|resolved| resolved.get("repo_code"))
            .or_else(|| owner_repo.get("repo_code"))
            .filter(|value| !value.is_null())
        {
            document.insert(
                "owner_repo_display_string_u_ssort".to_string(),
                repo_code.clone(),
            );
        }
    }
    if let Some(location_profile) = record
        .get("location_profile")
        .filter(|value| ruby_truthy(Some(value)))
    {
        let location_profile = location_profile
            .as_object()
            .ok_or_else(|| error("field 'location_profile' must be an object"))?;
        document.insert(
            "location_profile_uri_u_sstr".to_string(),
            location_profile.get("ref").cloned().unwrap_or(Value::Null),
        );
        let resolved = location_profile
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("location_profile must contain _resolved"))?;
        document.insert(
            "location_profile_display_string_u_ssort".to_string(),
            resolved
                .get("display_string")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    Ok(())
}

fn clean_display_string_for_sort(record: &Map<String, Value>) -> Result<Value, IndexDocumentError> {
    match record.get("display_string") {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(value)) => Ok(Value::String(clean_for_sort(value))),
        Some(_) => Err(error("field 'display_string' must be a string")),
    }
}

fn add_container_profile_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let display_string = record.get("display_string").cloned().unwrap_or(Value::Null);
    let title_sort = clean_display_string_for_sort(record)?;
    document.insert("title".to_string(), display_string.clone());
    document.insert("title_sort".to_string(), title_sort);
    document.insert("display_string".to_string(), display_string.clone());
    document.insert(
        "note".to_string(),
        record.get("note").cloned().unwrap_or(Value::Null),
    );
    for property in ["width", "height", "depth"] {
        document.insert(
            format!("container_profile_{property}_u_sstr"),
            record.get(property).cloned().unwrap_or(Value::Null),
        );
    }
    document.insert(
        "container_profile_dimension_units_u_sstr".to_string(),
        record
            .get("dimension_units")
            .cloned()
            .unwrap_or(Value::Null),
    );
    document.insert("typeahead_sort_key_u_sort".to_string(), display_string);
    Ok(())
}

fn add_location_profile_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let display_string = record.get("display_string").cloned().unwrap_or(Value::Null);
    let title_sort = clean_display_string_for_sort(record)?;
    document.insert("title".to_string(), display_string.clone());
    document.insert("title_sort".to_string(), title_sort);
    document.insert("display_string".to_string(), display_string.clone());
    for property in ["width", "height", "depth"] {
        document.insert(
            format!("location_profile_{property}_u_sstr"),
            record.get(property).cloned().unwrap_or(Value::Null),
        );
    }
    document.insert(
        "location_profile_dimension_units_u_sstr".to_string(),
        record
            .get("dimension_units")
            .cloned()
            .unwrap_or(Value::Null),
    );
    document.insert("typeahead_sort_key_u_sort".to_string(), display_string);
    Ok(())
}

fn parse_assessment_id(record: &Map<String, Value>) -> Result<u64, IndexDocumentError> {
    let uri = required_string(record.get("uri"), "uri")?;
    let without_fragment = uri.split('#').next().unwrap_or(uri);
    let Some((prefix, tail)) = without_fragment.rsplit_once("/assessments/") else {
        return Err(error("assessment uri must contain /assessments/<id>"));
    };
    let valid_prefix = prefix
        .strip_prefix("/repositories/")
        .is_some_and(|repo| !repo.is_empty() && !repo.contains('/') && !repo.contains(' '));
    if !valid_prefix {
        return Err(error(
            "assessment uri must match /repositories/<repo_id>/assessments/<id>",
        ));
    }
    if tail.is_empty() || !tail.chars().all(|character| character.is_ascii_digit()) {
        return Err(error("assessment uri must end with a numeric id"));
    }
    tail.parse::<u64>()
        .map_err(|_| error("assessment uri id must be a valid integer"))
}

fn generate_years_for_date_range(
    begin_date: Option<&Value>,
    end_date: Option<&Value>,
) -> Vec<Value> {
    if !ruby_truthy(begin_date) {
        return Vec::new();
    }
    let effective_end = if ruby_truthy(end_date) {
        end_date
    } else {
        begin_date
    };
    let (Some((begin, begin_width)), Some((end, _))) =
        (leading_year(begin_date), leading_year(effective_end))
    else {
        return Vec::new();
    };
    if end >= begin && end - begin <= 10_000 {
        let width = begin_width;
        (begin..=end)
            .map(|year| Value::String(format!("{year:0width$}")))
            .collect()
    } else {
        Vec::new()
    }
}

fn resolved_display_string_or_title(resolved: &Map<String, Value>) -> Value {
    resolved
        .get("display_string")
        .filter(|value| ruby_truthy(Some(value)))
        .or_else(|| resolved.get("title"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn assessment_attribute_labels(
    record: &Map<String, Value>,
    field: &str,
    include_notes: bool,
) -> Result<Vec<Value>, IndexDocumentError> {
    let values = match record.get(field) {
        None | Some(Value::Null) => &[][..],
        Some(value) => value
            .as_array()
            .ok_or_else(|| error(format!("field '{field}' must be an array")))?,
    };
    let mut labels = Vec::new();
    for entry in values {
        let entry = entry
            .as_object()
            .ok_or_else(|| error(format!("{field} entries must be objects")))?;
        if entry.contains_key("value") || (include_notes && entry.contains_key("note")) {
            labels.push(entry.get("label").cloned().unwrap_or(Value::Null));
        }
    }
    Ok(labels)
}

fn add_assessment_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let assessment_id = parse_assessment_id(record)?;
    document.insert(
        "assessment_id".to_string(),
        Value::Number(assessment_id.into()),
    );
    let display_string = record.get("display_string").cloned().unwrap_or(Value::Null);
    document.insert("title".to_string(), display_string.clone());
    document.insert("display_string".to_string(), display_string);

    let mut record_uris = Vec::new();
    let mut records = Vec::new();
    let mut record_types = Vec::new();
    for linked_record in wrapped_values(record.get("records")) {
        let linked_record = linked_record
            .as_object()
            .ok_or_else(|| error("records entries must be objects"))?;
        record_uris.push(linked_record.get("ref").cloned().unwrap_or(Value::Null));
        let resolved = linked_record
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("records entries must contain _resolved"))?;
        records.push(resolved_display_string_or_title(resolved));
        let record_type = required_string(
            resolved.get("jsonmodel_type"),
            "records._resolved.jsonmodel_type",
        )?;
        record_types.push(record_type.to_string());
    }
    record_types.sort();
    record_types.dedup();
    document.insert(
        "assessment_record_uris".to_string(),
        Value::Array(record_uris),
    );
    document.insert("assessment_records".to_string(), Value::Array(records));
    document.insert(
        "assessment_record_types".to_string(),
        Value::Array(record_types.into_iter().map(Value::String).collect()),
    );

    let mut surveyor_uris = Vec::new();
    let mut surveyors = Vec::new();
    for surveyor in wrapped_values(record.get("surveyed_by")) {
        let surveyor = surveyor
            .as_object()
            .ok_or_else(|| error("surveyed_by entries must be objects"))?;
        surveyor_uris.push(surveyor.get("ref").cloned().unwrap_or(Value::Null));
        let resolved = surveyor
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("surveyed_by entries must contain _resolved"))?;
        surveyors.push(resolved.get("title").cloned().unwrap_or(Value::Null));
    }
    document.insert(
        "assessment_surveyor_uris".to_string(),
        Value::Array(surveyor_uris),
    );
    document.insert("assessment_surveyors".to_string(), Value::Array(surveyors));

    document.insert(
        "assessment_survey_begin".to_string(),
        Value::String(format!(
            "{}T00:00:00Z",
            ruby_string(record.get("survey_begin"))
        )),
    );
    if let Some(survey_end) = record
        .get("survey_end")
        .filter(|value| ruby_truthy(Some(value)))
    {
        document.insert(
            "assessment_survey_end".to_string(),
            Value::String(format!("{}T00:00:00Z", ruby_string(Some(survey_end)))),
        );
    }
    document.insert(
        "assessment_review_required".to_string(),
        Value::Bool(ruby_truthy(record.get("review_required"))),
    );
    document.insert(
        "assessment_sensitive_material".to_string(),
        Value::Bool(ruby_truthy(record.get("sensitive_material"))),
    );

    let reviewers = wrapped_values(record.get("reviewer"));
    if !reviewers.is_empty() {
        let mut reviewer_uris = Vec::new();
        let mut reviewer_titles = Vec::new();
        for reviewer in reviewers {
            let reviewer = reviewer
                .as_object()
                .ok_or_else(|| error("reviewer entries must be objects"))?;
            reviewer_uris.push(reviewer.get("ref").cloned().unwrap_or(Value::Null));
            let resolved = reviewer
                .get("_resolved")
                .and_then(Value::as_object)
                .ok_or_else(|| error("reviewer entries must contain _resolved"))?;
            reviewer_titles.push(resolved.get("title").cloned().unwrap_or(Value::Null));
        }
        document.insert(
            "assessment_reviewer_uris".to_string(),
            Value::Array(reviewer_uris),
        );
        document.insert(
            "assessment_reviewers".to_string(),
            Value::Array(reviewer_titles),
        );
    }

    document.insert(
        "assessment_inactive".to_string(),
        Value::Bool(ruby_truthy(record.get("inactive"))),
    );
    document.insert(
        "assessment_survey_year".to_string(),
        Value::Array(generate_years_for_date_range(
            record.get("survey_begin"),
            record.get("survey_end"),
        )),
    );

    let mut collection_uris = Vec::new();
    let mut collections = Vec::new();
    for collection in wrapped_values(record.get("collections")) {
        let collection = collection
            .as_object()
            .ok_or_else(|| error("collections entries must be objects"))?;
        collection_uris.push(collection.get("ref").cloned().unwrap_or(Value::Null));
        let resolved = collection
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("collections entries must contain _resolved"))?;
        collections.push(resolved_display_string_or_title(resolved));
    }
    document.insert(
        "assessment_collection_uris".to_string(),
        Value::Array(collection_uris),
    );
    document.insert(
        "assessment_collections".to_string(),
        Value::Array(collections),
    );

    document.insert(
        "assessment_completed".to_string(),
        Value::Bool(
            record
                .get("survey_end")
                .is_some_and(|value| !value.is_null()),
        ),
    );
    document.insert(
        "assessment_formats".to_string(),
        Value::Array(assessment_attribute_labels(record, "formats", false)?),
    );
    document.insert(
        "assessment_ratings".to_string(),
        Value::Array(assessment_attribute_labels(record, "ratings", true)?),
    );
    document.insert(
        "assessment_conservation_issues".to_string(),
        Value::Array(assessment_attribute_labels(
            record,
            "conservation_issues",
            false,
        )?),
    );
    document.insert(
        "title_sort".to_string(),
        Value::String(format!("{assessment_id:010}")),
    );
    Ok(())
}

fn is_ruby_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character as u32,
            0xa1 | 0xa7 | 0xab | 0xb6..=0xb7 | 0xbb
                | 0xbf | 0x37e | 0x387 | 0x55a..=0x55f | 0x589..=0x58a
                | 0x5be | 0x5c0 | 0x5c3 | 0x5c6 | 0x5f3..=0x5f4
                | 0x609..=0x60a | 0x60c..=0x60d | 0x61b | 0x61d..=0x61f | 0x66a..=0x66d
                | 0x6d4 | 0x700..=0x70d | 0x7f7..=0x7f9 | 0x830..=0x83e | 0x85e
                | 0x964..=0x965 | 0x970 | 0x9fd | 0xa76 | 0xaf0
                | 0xc77 | 0xc84 | 0xdf4 | 0xe4f | 0xe5a..=0xe5b
                | 0xf04..=0xf12 | 0xf14 | 0xf3a..=0xf3d | 0xf85 | 0xfd0..=0xfd4
                | 0xfd9..=0xfda | 0x104a..=0x104f | 0x10fb | 0x1360..=0x1368 | 0x1400
                | 0x166e | 0x169b..=0x169c | 0x16eb..=0x16ed | 0x1735..=0x1736 | 0x17d4..=0x17d6
                | 0x17d8..=0x17da | 0x1800..=0x180a | 0x1944..=0x1945 | 0x1a1e..=0x1a1f | 0x1aa0..=0x1aa6
                | 0x1aa8..=0x1aad | 0x1b5a..=0x1b60 | 0x1b7d..=0x1b7e | 0x1bfc..=0x1bff | 0x1c3b..=0x1c3f
                | 0x1c7e..=0x1c7f | 0x1cc0..=0x1cc7 | 0x1cd3 | 0x2010..=0x2027 | 0x2030..=0x2043
                | 0x2045..=0x2051 | 0x2053..=0x205e | 0x207d..=0x207e | 0x208d..=0x208e | 0x2308..=0x230b
                | 0x2329..=0x232a | 0x2768..=0x2775 | 0x27c5..=0x27c6 | 0x27e6..=0x27ef | 0x2983..=0x2998
                | 0x29d8..=0x29db | 0x29fc..=0x29fd | 0x2cf9..=0x2cfc | 0x2cfe..=0x2cff | 0x2d70
                | 0x2e00..=0x2e2e | 0x2e30..=0x2e4f | 0x2e52..=0x2e5d | 0x3001..=0x3003 | 0x3008..=0x3011
                | 0x3014..=0x301f | 0x3030 | 0x303d | 0x30a0 | 0x30fb
                | 0xa4fe..=0xa4ff | 0xa60d..=0xa60f | 0xa673 | 0xa67e | 0xa6f2..=0xa6f7
                | 0xa874..=0xa877 | 0xa8ce..=0xa8cf | 0xa8f8..=0xa8fa | 0xa8fc | 0xa92e..=0xa92f
                | 0xa95f | 0xa9c1..=0xa9cd | 0xa9de..=0xa9df | 0xaa5c..=0xaa5f | 0xaade..=0xaadf
                | 0xaaf0..=0xaaf1 | 0xabeb | 0xfd3e..=0xfd3f | 0xfe10..=0xfe19 | 0xfe30..=0xfe52
                | 0xfe54..=0xfe61 | 0xfe63 | 0xfe68 | 0xfe6a..=0xfe6b | 0xff01..=0xff03
                | 0xff05..=0xff0a | 0xff0c..=0xff0f | 0xff1a..=0xff1b | 0xff1f..=0xff20 | 0xff3b..=0xff3d
                | 0xff3f | 0xff5b | 0xff5d | 0xff5f..=0xff65 | 0x10100..=0x10102
                | 0x1039f | 0x103d0 | 0x1056f | 0x10857 | 0x1091f
                | 0x1093f | 0x10a50..=0x10a58 | 0x10a7f | 0x10af0..=0x10af6 | 0x10b39..=0x10b3f
                | 0x10b99..=0x10b9c | 0x10ead | 0x10f55..=0x10f59 | 0x10f86..=0x10f89 | 0x11047..=0x1104d
                | 0x110bb..=0x110bc | 0x110be..=0x110c1 | 0x11140..=0x11143 | 0x11174..=0x11175 | 0x111c5..=0x111c8
                | 0x111cd | 0x111db | 0x111dd..=0x111df | 0x11238..=0x1123d | 0x112a9
                | 0x1144b..=0x1144f | 0x1145a..=0x1145b | 0x1145d | 0x114c6 | 0x115c1..=0x115d7
                | 0x11641..=0x11643 | 0x11660..=0x1166c | 0x116b9 | 0x1173c..=0x1173e | 0x1183b
                | 0x11944..=0x11946 | 0x119e2 | 0x11a3f..=0x11a46 | 0x11a9a..=0x11a9c | 0x11a9e..=0x11aa2
                | 0x11b00..=0x11b09 | 0x11c41..=0x11c45 | 0x11c70..=0x11c71 | 0x11ef7..=0x11ef8 | 0x11f43..=0x11f4f
                | 0x11fff | 0x12470..=0x12474 | 0x12ff1..=0x12ff2 | 0x16a6e..=0x16a6f | 0x16af5
                | 0x16b37..=0x16b3b | 0x16b44 | 0x16e97..=0x16e9a | 0x16fe2 | 0x1bc9f
                | 0x1da87..=0x1da8b | 0x1e95e..=0x1e95f
        )
}

fn generate_permutations_for_identifier(
    identifier: Option<&Value>,
    path: &str,
) -> Result<Vec<Value>, IndexDocumentError> {
    let identifier = match identifier {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(value)) => value.as_str(),
        Some(_) => return Err(error(format!("field '{path}' must be a string"))),
    };

    let mut punct_to_space = String::with_capacity(identifier.len());
    let mut previous_was_punct = false;
    for character in identifier.chars() {
        if is_ruby_punctuation(character) {
            if !previous_was_punct {
                punct_to_space.push(' ');
                previous_was_punct = true;
            }
        } else {
            punct_to_space.push(character);
            previous_was_punct = false;
        }
    }

    let without_punct_or_space: String = identifier
        .chars()
        .filter(|character| !is_ruby_punctuation(*character) && *character != ' ')
        .collect();

    let mut digit_split = String::with_capacity(identifier.len());
    let mut previous_was_digit: Option<bool> = None;
    for character in identifier.chars() {
        let is_digit = character.is_ascii_digit();
        if let Some(previous) = previous_was_digit {
            if previous != is_digit {
                digit_split.push(' ');
            }
        }
        digit_split.push(character);
        previous_was_digit = Some(is_digit);
    }

    let mut permutations = Vec::with_capacity(4);
    for candidate in [
        identifier.to_string(),
        punct_to_space,
        without_punct_or_space,
        digit_split,
    ] {
        push_unique(&mut permutations, Value::String(candidate));
    }
    Ok(permutations)
}

fn add_top_container_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let display_string =
        required_string(record.get("display_string"), "display_string")?.to_string();
    let typeahead: String = display_string
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect();

    document.insert(
        "title".to_string(),
        record
            .get("long_display_string")
            .cloned()
            .unwrap_or(Value::Null),
    );
    document.insert(
        "display_string".to_string(),
        Value::String(display_string.clone()),
    );
    document.insert(
        "type_u_ssort".to_string(),
        record.get("type").cloned().unwrap_or(Value::Null),
    );
    document.insert(
        "notes".to_string(),
        record.get("internal_note").cloned().unwrap_or(Value::Null),
    );

    if let Some(series_value) = record
        .get("series")
        .filter(|value| ruby_truthy(Some(value)))
    {
        let series_list = series_value
            .as_array()
            .ok_or_else(|| error("field 'series' must be an array"))?;
        let mut series_uris = Vec::with_capacity(series_list.len());
        let mut series_titles = Vec::with_capacity(series_list.len());
        let mut series_levels = Vec::with_capacity(series_list.len());
        let mut series_identifiers_stored = Vec::with_capacity(series_list.len());
        let mut series_identifiers_text = Vec::new();
        let mut published_series_uris = Vec::new();
        let mut published_series_titles = Vec::new();

        for series in series_list {
            let series = series
                .as_object()
                .ok_or_else(|| error("series entries must be objects"))?;
            let uri = series.get("ref").cloned().unwrap_or(Value::Null);
            let title = series.get("display_string").cloned().unwrap_or(Value::Null);
            series_uris.push(uri.clone());
            series_titles.push(title.clone());
            series_levels.push(
                series
                    .get("level_display_string")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            series_identifiers_stored
                .push(series.get("identifier").cloned().unwrap_or(Value::Null));
            series_identifiers_text.extend(generate_permutations_for_identifier(
                series.get("identifier"),
                "series.identifier",
            )?);
            if ruby_truthy(series.get("publish")) {
                published_series_uris.push(uri);
                published_series_titles.push(title);
            }
        }

        document.insert("series_uri_u_sstr".to_string(), Value::Array(series_uris));
        document.insert(
            "series_title_u_sstr".to_string(),
            Value::Array(series_titles),
        );
        document.insert(
            "series_level_u_sstr".to_string(),
            Value::Array(series_levels),
        );
        document.insert(
            "series_identifier_stored_u_sstr".to_string(),
            Value::Array(series_identifiers_stored),
        );
        document.insert(
            "series_identifier_u_stext".to_string(),
            Value::Array(series_identifiers_text),
        );
        if !published_series_uris.is_empty() {
            document.insert(
                "published_series_uri_u_sstr".to_string(),
                Value::Array(published_series_uris),
            );
            document.insert(
                "published_series_title_u_sstr".to_string(),
                Value::Array(published_series_titles),
            );
        }
    }

    let collection_is_empty = if let Some(collection_value) = record
        .get("collection")
        .filter(|value| ruby_truthy(Some(value)))
    {
        let collections = collection_value
            .as_array()
            .ok_or_else(|| error("field 'collection' must be an array"))?;
        let mut collection_uris = Vec::with_capacity(collections.len());
        let mut collection_displays = Vec::with_capacity(collections.len());
        let mut collection_identifiers_stored = Vec::with_capacity(collections.len());
        let mut collection_identifiers_text = Vec::new();

        for collection in collections {
            let collection = collection
                .as_object()
                .ok_or_else(|| error("collection entries must be objects"))?;
            collection_uris.push(collection.get("ref").cloned().unwrap_or(Value::Null));
            collection_displays.push(
                collection
                    .get("display_string")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            collection_identifiers_stored
                .push(collection.get("identifier").cloned().unwrap_or(Value::Null));
            collection_identifiers_text.extend(generate_permutations_for_identifier(
                collection.get("identifier"),
                "collection.identifier",
            )?);
        }

        document.insert(
            "collection_uri_u_sstr".to_string(),
            Value::Array(collection_uris),
        );
        document.insert(
            "collection_display_string_u_sstr".to_string(),
            Value::Array(collection_displays),
        );
        document.insert(
            "collection_identifier_stored_u_sstr".to_string(),
            Value::Array(collection_identifiers_stored),
        );
        document.insert(
            "collection_identifier_u_stext".to_string(),
            Value::Array(collection_identifiers_text),
        );
        collections.is_empty()
    } else {
        true
    };

    if let Some(container_profile) = record
        .get("container_profile")
        .filter(|value| ruby_truthy(Some(value)))
    {
        let container_profile = container_profile
            .as_object()
            .ok_or_else(|| error("field 'container_profile' must be an object"))?;
        document.insert(
            "container_profile_uri_u_sstr".to_string(),
            container_profile.get("ref").cloned().unwrap_or(Value::Null),
        );
        let resolved = container_profile
            .get("_resolved")
            .and_then(Value::as_object)
            .ok_or_else(|| error("container_profile must contain _resolved"))?;
        document.insert(
            "container_profile_display_string_u_sstr".to_string(),
            resolved
                .get("display_string")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }

    let container_locations = match record.get("container_locations") {
        None | Some(Value::Null) => &[][..],
        Some(value) => value
            .as_array()
            .ok_or_else(|| error("field 'container_locations' must be an array"))?,
    };
    if !container_locations.is_empty() {
        let mut location_uri_u_sstr = Vec::new();
        let mut location_uris = Vec::new();
        let mut location_display_strings = Vec::new();
        let mut location_profile_display_strings = Vec::new();

        for container_location in container_locations {
            let container_location = container_location
                .as_object()
                .ok_or_else(|| error("container_locations entries must be objects"))?;
            if container_location.get("status").and_then(Value::as_str) == Some("current") {
                let reference = container_location
                    .get("ref")
                    .cloned()
                    .unwrap_or(Value::Null);
                location_uri_u_sstr.push(reference.clone());
                location_uris.push(reference);
                let resolved = container_location
                    .get("_resolved")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        error("container_locations current entries must contain _resolved")
                    })?;
                location_display_strings
                    .push(resolved.get("title").cloned().unwrap_or(Value::Null));
                if let Some(location_profile) = resolved
                    .get("location_profile")
                    .filter(|value| ruby_truthy(Some(value)))
                {
                    let location_profile = location_profile.as_object().ok_or_else(|| {
                        error("container_locations._resolved.location_profile must be an object")
                    })?;
                    if let Some(lp_resolved) = location_profile
                        .get("_resolved")
                        .filter(|value| ruby_truthy(Some(value)))
                    {
                        let lp_resolved = lp_resolved.as_object().ok_or_else(|| {
                            error(
                                "container_locations._resolved.location_profile._resolved must be an object",
                            )
                        })?;
                        push_unique(
                            &mut location_profile_display_strings,
                            lp_resolved
                                .get("display_string")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                    }
                }
            }
        }

        document.insert("has_location_u_sbool".to_string(), Value::Bool(true));
        document.insert(
            "location_uri_u_sstr".to_string(),
            Value::Array(location_uri_u_sstr),
        );
        document.insert("location_uris".to_string(), Value::Array(location_uris));
        document.insert(
            "location_display_string_u_sstr".to_string(),
            Value::Array(location_display_strings),
        );
        document.insert(
            "location_profile_display_string_u_sstr".to_string(),
            Value::Array(location_profile_display_strings),
        );
    } else {
        document.insert("has_location_u_sbool".to_string(), Value::Bool(false));
    }

    document.insert(
        "exported_u_sbool".to_string(),
        Value::Bool(record.contains_key("exported_to_ils")),
    );
    document.insert(
        "empty_u_sbool".to_string(),
        Value::Bool(collection_is_empty),
    );

    if let Some(indicator) = record
        .get("indicator")
        .filter(|value| ruby_truthy(Some(value)))
    {
        document.insert("indicator_u_icusort".to_string(), indicator.clone());
    }

    document.insert(
        "top_container_u_typeahead_utext".to_string(),
        Value::String(typeahead),
    );
    document.insert(
        "top_container_u_icusort".to_string(),
        Value::String(display_string),
    );
    let barcode = record.get("barcode").cloned().unwrap_or(Value::Null);
    document.insert("barcode_u_sstr".to_string(), barcode.clone());
    document.insert("barcode_u_icusort".to_string(), barcode);
    for (document_field, record_field) in [
        ("ils_holding_id_u_sstr", "ils_holding_id"),
        ("subcontainer_barcodes_u_sstr", "subcontainer_barcodes"),
        ("created_for_collection_u_sstr", "created_for_collection"),
    ] {
        document.insert(
            document_field.to_string(),
            record.get(record_field).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(())
}

fn add_type_specific_fields(
    document: &mut Map<String, Value>,
    record_type: &str,
    record: &Map<String, Value>,
) -> Result<Vec<Value>, IndexDocumentError> {
    let mut markers = Vec::new();
    match record_type {
        "archival_object" => {
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
            markers.push(json_string("resource_archival_object_metadata"));
        }
        "resource" => {
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
            document.insert(
                "related_accession_uris".to_string(),
                Value::Array(related_uris(record, "related_accessions", true, true)?),
            );
            add_collection_management_fields(document, record);
            markers.push(json_string("resource_archival_object_metadata"));
        }
        "accession" => {
            for field in [
                "acquisition_type",
                "resource_type",
                "restrictions_apply",
                "access_restrictions",
                "use_restrictions",
                "slug",
                "is_slug_auto",
            ] {
                document.insert(
                    field.to_string(),
                    record.get(field).cloned().unwrap_or(Value::Null),
                );
            }
            document.insert(
                "title".to_string(),
                record.get("display_string").cloned().unwrap_or(Value::Null),
            );
            document.insert(
                "identifier".to_string(),
                Value::String(joined_identifier(record, "-")),
            );
            document.insert(
                "related_resource_uris".to_string(),
                Value::Array(related_uris(record, "related_resources", true, true)?),
            );
            document.insert(
                "related_accession_uris".to_string(),
                Value::Array(related_uris(record, "related_accessions", true, true)?),
            );
            add_collection_management_fields(document, record);
            markers.push(json_string("accession_digital_object_metadata"));
        }
        "digital_object" => {
            for field in [
                "digital_object_type",
                "level",
                "restrictions",
                "slug",
                "is_slug_auto",
            ] {
                document.insert(
                    field.to_string(),
                    record.get(field).cloned().unwrap_or(Value::Null),
                );
            }
            let identifier = record
                .get("digital_object_id")
                .cloned()
                .unwrap_or(Value::Null);
            document.insert("digital_object_id".to_string(), identifier.clone());
            document.insert("identifier".to_string(), identifier);
            document.insert(
                "collection_uri_u_sstr".to_string(),
                Value::Array(related_uris(record, "collection", false, false)?),
            );
            document.insert(
                "linked_instance_uris".to_string(),
                Value::Array(related_uris(record, "linked_instances", true, true)?),
            );
            markers.push(json_string("accession_digital_object_metadata"));
        }
        "subject" => {
            add_subject_fields(document, record)?;
            markers.push(json_string("agent_subject_metadata"));
        }
        "agent_person" | "agent_family" | "agent_software" | "agent_corporate_entity" => {
            add_agent_fields(document, record)?;
            markers.push(json_string("agent_subject_metadata"));
        }
        "digital_object_component" => {
            add_digital_object_component_fields(document, record)?;
            markers.push(json_string("component_classification_metadata"));
        }
        "classification" | "classification_term" => {
            add_classification_fields(document, record_type, record)?;
            markers.push(json_string("component_classification_metadata"));
        }
        "repository" => {
            add_repository_fields(document, record)?;
            markers.push(json_string("repository_event_location_metadata"));
        }
        "event" => {
            add_event_fields(document, record)?;
            markers.push(json_string("repository_event_location_metadata"));
        }
        "location" => {
            add_location_fields(document, record)?;
            markers.push(json_string("repository_event_location_metadata"));
        }
        "container_profile" => {
            add_container_profile_fields(document, record)?;
            markers.push(json_string("profile_assessment_metadata"));
        }
        "location_profile" => {
            add_location_profile_fields(document, record)?;
            markers.push(json_string("profile_assessment_metadata"));
        }
        "assessment" => {
            add_assessment_fields(document, record)?;
            markers.push(json_string("profile_assessment_metadata"));
        }
        "top_container" => {
            add_top_container_fields(document, record)?;
            markers.push(json_string("top_container_metadata"));
        }
        _ => return Ok(markers),
    }

    add_external_ids(document, record)?;
    if matches!(record_type, "resource" | "accession" | "digital_object") {
        add_classifications(document, record)?;
    }
    if matches!(record_type, "resource" | "archival_object" | "accession") {
        add_instance_fields(document, record)?;
    }
    let four_part_id = joined_identifier(record, " ");
    if !four_part_id.is_empty() {
        document.insert("four_part_id".to_string(), Value::String(four_part_id));
    }
    add_ancestors(document, record)?;
    add_rights_statement_agents(document, record)?;
    add_languages(document, record)?;
    Ok(markers)
}

fn add_record_type_markers(
    document: &mut Map<String, Value>,
    record_type: &str,
    record: &Map<String, Value>,
    rust_fields: &mut Vec<Value>,
) -> Result<(), IndexDocumentError> {
    for marker in add_type_specific_fields(document, record_type, record)? {
        if !rust_fields.contains(&marker) {
            rust_fields.push(marker);
        }
    }
    Ok(())
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
    add_record_type_markers(document, record_type, record, &mut rust_fields)?;
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
    fn builds_accession_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/accessions/4".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/4",
                "title": "Original accession title",
                "display_string": "2026-004: New records",
                "publish": true,
                "id_0": "2026",
                "id_1": "004",
                "acquisition_type": "gift",
                "resource_type": "papers",
                "restrictions_apply": true,
                "access_restrictions": true,
                "use_restrictions": false,
                "related_resources": [
                    {"ref": "/repositories/2/resources/7"},
                    {"ref": "/repositories/2/resources/7"}
                ],
                "related_accessions": [{"ref": "/repositories/2/accessions/3"}],
                "slug": "2026-004-new-records",
                "is_slug_auto": true,
                "collection_management": {
                    "processing_priority": "medium",
                    "processors": "Archive team"
                },
                "external_ids": [{"external_id": "legacy-a4"}],
                "classifications": [{
                    "ref": "/classifications/2",
                    "_resolved": {"path_from_root": [{"identifier": "ACC"}]}
                }],
                "instances": [{
                    "sub_container": {
                        "top_container": {"ref": "/repositories/2/top_containers/6"}
                    }
                }],
                "ancestors": [],
                "lang_materials": [{"language_and_script": {"language": "gle"}}]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["title"], "2026-004: New records");
        assert_eq!(document["identifier"], "2026-004");
        assert_eq!(document["four_part_id"], "2026 004");
        assert_eq!(document["acquisition_type"], "gift");
        assert_eq!(document["restrictions_apply"], true);
        assert_eq!(
            document["related_resource_uris"],
            json!(["/repositories/2/resources/7"])
        );
        assert_eq!(
            document["related_accession_uris"],
            json!(["/repositories/2/accessions/3"])
        );
        assert_eq!(document["processing_priority"], "medium");
        assert_eq!(
            document["top_container_uri_u_sstr"],
            json!(["/repositories/2/top_containers/6"])
        );
        assert_eq!(document["langcode"], json!(["gle"]));
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "accession_digital_object_metadata"
            ])
        );
    }

    #[test]
    fn builds_digital_object_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/digital_objects/5".to_string(),
            record_type: "digital_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/digital_objects/5",
                "title": "Scanned register",
                "publish": true,
                "digital_object_type": "still_image",
                "digital_object_id": "DO-5",
                "level": "image",
                "restrictions": false,
                "slug": "scanned-register",
                "is_slug_auto": false,
                "collection": [{"ref": "/repositories/2/resources/7"}],
                "linked_instances": [
                    {"ref": "/repositories/2/archival_objects/8"},
                    {"ref": "/repositories/2/archival_objects/8"}
                ],
                "external_ids": [],
                "classifications": [],
                "ancestors": [],
                "lang_materials": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["digital_object_type"], "still_image");
        assert_eq!(document["digital_object_id"], "DO-5");
        assert_eq!(document["identifier"], "DO-5");
        assert_eq!(document["level"], "image");
        assert_eq!(
            document["collection_uri_u_sstr"],
            json!(["/repositories/2/resources/7"])
        );
        assert_eq!(
            document["linked_instance_uris"],
            json!(["/repositories/2/archival_objects/8"])
        );
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "accession_digital_object_metadata"
            ])
        );
    }

    #[test]
    fn builds_subject_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/subjects/12".to_string(),
            record_type: "subject".to_string(),
            repository: Value::Null,
            record: json!({
                "uri": "/subjects/12",
                "title": "Monastic archives",
                "publish": true,
                "is_linked_to_published_record": true,
                "source": "local",
                "terms": [{"term": "Monastic archives", "term_type": "topical"}],
                "slug": "monastic-archives",
                "is_slug_auto": true,
                "used_within_repositories": ["/repositories/2"],
                "used_within_published_repositories": ["/repositories/2"],
                "external_ids": [{"external_id": "subject-12"}]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["source"], "local");
        assert_eq!(document["first_term_type"], "topical");
        assert_eq!(document["publish"], true);
        assert_eq!(
            document["used_within_repository"],
            json!(["/repositories/2"])
        );
        assert_eq!(document["external_id"], json!(["subject-12"]));
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!(["common_metadata", "fullrecord", "agent_subject_metadata"])
        );
    }

    #[test]
    fn builds_agent_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/agents/people/9".to_string(),
            record_type: "agent_person".to_string(),
            repository: Value::Null,
            record: json!({
                "uri": "/agents/people/9",
                "title": "Original agent title",
                "publish": true,
                "display_name": {"sort_name": "O'Connor, Maeve"},
                "names": [{
                    "authorized": true,
                    "authority_id": "name-authority",
                    "source": "local",
                    "rules": "local"
                }],
                "agent_record_identifiers": [{
                    "primary_identifier": true,
                    "record_identifier": "IE-AUTH-9",
                    "source": "naf"
                }],
                "agent_conventions_declarations": [{"name_rule": "rda"}],
                "linked_agent_roles": ["creator", "subject"],
                "related_agents": [{"ref": "/agents/families/3"}],
                "slug": "oconnor-maeve",
                "is_slug_auto": false,
                "is_user": true,
                "agent_functions": [{
                    "subjects": [{
                        "ref": "/subjects/1",
                        "_resolved": {"title": "Archives"}
                    }],
                    "places": [{
                        "ref": "/subjects/2",
                        "_resolved": {"title": "Ireland"}
                    }]
                }],
                "agent_occupations": [],
                "agent_places": [],
                "agent_topics": [],
                "agent_resources": [],
                "used_within_repositories": ["/repositories/2"],
                "used_within_published_repositories": ["/repositories/2"],
                "external_ids": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["title"], "O'Connor, Maeve");
        assert_eq!(document["authority_id"], "IE-AUTH-9");
        assert_eq!(document["source"], "naf");
        assert_eq!(document["rules"], "rda");
        assert_eq!(
            document["related_agent_uris"],
            json!(["/agents/families/3"])
        );
        assert_eq!(document["is_user"], true);
        assert_eq!(
            document["types"],
            json!(["agent_person", "agent_with_user", "agent"])
        );
        assert_eq!(document["subjects"], json!(["Archives", "Ireland"]));
        assert_eq!(
            document["subject_uris"],
            json!(["/subjects/1", "/subjects/2"])
        );
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!(["common_metadata", "fullrecord", "agent_subject_metadata"])
        );
    }

    #[test]
    fn builds_digital_object_component_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/digital_object_components/4".to_string(),
            record_type: "digital_object_component".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/digital_object_components/4",
                "title": "Original title",
                "display_string": "Chapter one",
                "publish": true,
                "component_id": "DOC-4",
                "digital_object": {"ref": "/repositories/2/digital_objects/1"},
                "slug": "chapter-one",
                "is_slug_auto": true,
                "external_ids": [{"external_id": "component-4"}],
                "ancestors": [{"ref": "/repositories/2/digital_objects/1"}]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["title"], "Chapter one");
        assert_eq!(
            document["digital_object"],
            "/repositories/2/digital_objects/1"
        );
        assert_eq!(document["digital_object_id"], "DOC-4");
        assert_eq!(document["identifier"], "DOC-4");
        assert_eq!(document["external_id"], json!(["component-4"]));
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "component_classification_metadata"
            ])
        );
    }

    #[test]
    fn builds_classification_term_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/classification_terms/8".to_string(),
            record_type: "classification_term".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/classification_terms/8",
                "title": "Annual reports",
                "publish": true,
                "identifier": "RG123A",
                "path_from_root": [{"identifier": "RG", "title": "Records"}],
                "creator": {
                    "ref": "/agents/corporate_entities/3",
                    "_resolved": {
                        "publish": true,
                        "display_name": {"sort_name": "Example Organisation"}
                    }
                },
                "repository": {"_resolved": {"display_string": "Example Archives"}},
                "has_classification_terms": false,
                "slug": "annual-reports",
                "is_slug_auto": true,
                "classification": {"ref": "/repositories/2/classifications/1"},
                "external_ids": [],
                "ancestors": [{"ref": "/repositories/2/classifications/1"}]
            }),
            enrich: true,
        })
        .unwrap();

        let expected_letters = format!("{:<255}", "RG").replace(' ', "#");
        let expected_numbers = format!("{:0>255}", "123");
        let expected_rest = format!("{:<255}", "A");
        let expected_sort = format!("{expected_letters}{expected_numbers}{expected_rest}");
        assert_eq!(document["identifier_sort"], expected_sort);
        assert_eq!(document["agents"], json!(["Example Organisation"]));
        assert_eq!(
            document["published_agent_uris"],
            json!(["/agents/corporate_entities/3"])
        );
        assert_eq!(
            document["classification"],
            "/repositories/2/classifications/1"
        );
        assert_eq!(document["repo_sort"], "Example Archives");
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "component_classification_metadata"
            ])
        );
    }

    #[test]
    fn builds_repository_specific_staff_index_fields() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/repositories/2".to_string(),
            record_type: "repository".to_string(),
            repository: Value::String("global".to_string()),
            record: json!({
                "uri": "/repositories/2",
                "repo_code": "ARCH-2",
                "display_string": "<em>St. Mary's</em> — Diocesan-Archives!",
                "publish": true,
                "slug": "st-marys-diocesan-archives",
                "is_slug_auto": false,
                "position": 4,
                "external_ids": [{"external_id": "repo-ext-2"}],
                "ancestors": [],
                "lang_materials": [{"language_and_script": {"language": "eng"}}]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["id"], "/repositories/2");
        assert_eq!(document["repository"], "/repositories/2");
        assert_eq!(document["title"], "ARCH-2");
        assert_eq!(document["title_sort"], "St Marys Diocesan Archives");
        assert_eq!(
            document["repo_sort"],
            "<em>St. Mary's</em> — Diocesan-Archives!"
        );
        assert_eq!(document["slug"], "st-marys-diocesan-archives");
        assert_eq!(document["is_slug_auto"], false);
        assert_eq!(document["position_int_sort"], 4);
        assert_eq!(document["external_id"], json!(["repo-ext-2"]));
        assert_eq!(document["ancestors"], json!([]));
        assert_eq!(document["langcode"], json!(["eng"]));
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "repository_event_location_metadata"
            ])
        );
    }

    #[test]
    fn builds_event_specific_staff_index_fields_for_all_linked_record_types() {
        let document = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/events/11".to_string(),
            record_type: "event".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/events/11",
                "event_type": "custody_transfer",
                "outcome": "pass",
                "publish": false,
                "linked_records": [
                    {
                        "ref": "/agents/people/1",
                        "_resolved": {
                            "jsonmodel_type": "agent_person",
                            "display_name": {"sort_name": "Person, One"}
                        }
                    },
                    {
                        "ref": "/agents/families/2",
                        "_resolved": {
                            "jsonmodel_type": "agent_family",
                            "display_name": {"sort_name": "Family, Two"}
                        }
                    },
                    {
                        "ref": "/agents/corporate_entities/3",
                        "_resolved": {
                            "jsonmodel_type": "agent_corporate_entity",
                            "display_name": {"sort_name": "Corporation, Three"}
                        }
                    },
                    {
                        "ref": "/agents/software/4",
                        "_resolved": {
                            "jsonmodel_type": "agent_software",
                            "display_name": {"sort_name": "Software, Four"}
                        }
                    },
                    {
                        "ref": "/repositories/2/accessions/5",
                        "_resolved": {
                            "jsonmodel_type": "accession",
                            "title": "Accession Five"
                        }
                    },
                    {
                        "ref": "/repositories/2/resources/6",
                        "_resolved": {
                            "jsonmodel_type": "resource",
                            "title": "Resource Six"
                        }
                    },
                    {
                        "ref": "/repositories/2/digital_objects/7",
                        "_resolved": {
                            "jsonmodel_type": "digital_object",
                            "title": "Digital Object Seven"
                        }
                    },
                    {
                        "ref": "/repositories/2/digital_object_components/8",
                        "_resolved": {
                            "jsonmodel_type": "digital_object_component",
                            "title": "Component Eight"
                        }
                    },
                    {
                        "ref": "/repositories/2/archival_objects/9",
                        "_resolved": {
                            "jsonmodel_type": "archival_object",
                            "display_string": "Archival Object Nine"
                        }
                    },
                    {
                        "ref": "/repositories/2/top_containers/10",
                        "_resolved": {
                            "jsonmodel_type": "top_container",
                            "display_string": "Box 10"
                        }
                    }
                ],
                "external_ids": [{"external_id": "event-11"}],
                "ancestors": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(document["event_type"], "custody_transfer");
        assert_eq!(document["title"], "custody_transfer");
        assert_eq!(document["outcome"], "pass");
        assert_eq!(
            document["linked_record_uris"],
            json!([
                "/agents/people/1",
                "/agents/families/2",
                "/agents/corporate_entities/3",
                "/agents/software/4",
                "/repositories/2/accessions/5",
                "/repositories/2/resources/6",
                "/repositories/2/digital_objects/7",
                "/repositories/2/digital_object_components/8",
                "/repositories/2/archival_objects/9",
                "/repositories/2/top_containers/10"
            ])
        );
        assert_eq!(
            document["linked_record_titles"],
            json!([
                "Person, One",
                "Family, Two",
                "Corporation, Three",
                "Software, Four",
                "Accession Five",
                "Resource Six",
                "Digital Object Seven",
                "Component Eight",
                "Archival Object Nine",
                "not_found"
            ])
        );
        assert_eq!(document["external_id"], json!(["event-11"]));
        assert_eq!(document["ancestors"], json!([]));
        assert_eq!(document["langcode"], json!([]));
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "repository_event_location_metadata"
            ])
        );
    }

    #[test]
    fn builds_location_specific_staff_index_fields_with_and_without_resolved_relations() {
        let resolved_location = build_index_document(IndexDocumentInput {
            uri: "/locations/15".to_string(),
            record_type: "location".to_string(),
            repository: Value::String("global".to_string()),
            record: json!({
                "uri": "/locations/15",
                "title": "Main Vault, Bay 3",
                "publish": false,
                "temporary": "conservation",
                "building": "Main Vault",
                "floor": "1",
                "room": "A",
                "area": "Bay 3",
                "owner_repo": {
                    "ref": "/repositories/2",
                    "_resolved": {"repo_code": "ARCH-2"}
                },
                "location_profile": {
                    "ref": "/location_profiles/4",
                    "_resolved": {"display_string": "Standard folio shelf"}
                },
                "external_ids": [{"external_id": "loc-15"}],
                "ancestors": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(resolved_location["temporary"], "conservation");
        assert_eq!(resolved_location["building"], "Main Vault");
        assert_eq!(resolved_location["floor"], "1");
        assert_eq!(resolved_location["room"], "A");
        assert_eq!(resolved_location["area"], "Bay 3");
        assert_eq!(
            resolved_location["owner_repo_uri_u_sstr"],
            "/repositories/2"
        );
        assert_eq!(
            resolved_location["owner_repo_display_string_u_ssort"],
            "ARCH-2"
        );
        assert_eq!(
            resolved_location["location_profile_uri_u_sstr"],
            "/location_profiles/4"
        );
        assert_eq!(
            resolved_location["location_profile_display_string_u_ssort"],
            "Standard folio shelf"
        );
        assert_eq!(resolved_location["external_id"], json!(["loc-15"]));
        assert_eq!(
            resolved_location["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "repository_event_location_metadata"
            ])
        );

        let unresolved_owner_repo = build_index_document(IndexDocumentInput {
            uri: "/locations/16".to_string(),
            record_type: "location".to_string(),
            repository: Value::String("global".to_string()),
            record: json!({
                "uri": "/locations/16",
                "title": "Annex, Room 2",
                "publish": false,
                "building": "Annex",
                "floor": Value::Null,
                "room": "2",
                "area": Value::Null,
                "owner_repo": {"ref": "/repositories/2"}
            }),
            enrich: true,
        })
        .unwrap();

        assert!(unresolved_owner_repo.get("temporary").is_none());
        assert_eq!(unresolved_owner_repo["building"], "Annex");
        assert_eq!(unresolved_owner_repo["floor"], Value::Null);
        assert_eq!(unresolved_owner_repo["room"], "2");
        assert_eq!(unresolved_owner_repo["area"], Value::Null);
        assert_eq!(
            unresolved_owner_repo["owner_repo_uri_u_sstr"],
            "/repositories/2"
        );
        assert!(unresolved_owner_repo
            .get("owner_repo_display_string_u_ssort")
            .is_none());
        assert!(unresolved_owner_repo
            .get("location_profile_uri_u_sstr")
            .is_none());
        assert!(unresolved_owner_repo
            .get("location_profile_display_string_u_ssort")
            .is_none());
    }

    #[test]
    fn builds_container_and_location_profile_staff_index_fields() {
        let container_profile = build_index_document(IndexDocumentInput {
            uri: "/container_profiles/3".to_string(),
            record_type: "container_profile".to_string(),
            repository: Value::String("global".to_string()),
            record: json!({
                "uri": "/container_profiles/3",
                "name": "Standard Hollinger",
                "display_string": "<b>Standard-Hollinger</b> [10x12x15] (inches)",
                "note": "Acid-free archival carton",
                "notes": "Schema string notes field",
                "width": "10",
                "height": "12",
                "depth": "15",
                "dimension_units": "inches",
                "external_ids": [{"external_id": "cp-3"}],
                "ancestors": [],
                "lang_materials": [{"language_and_script": {"language": "eng"}}]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(
            container_profile["title"],
            "<b>Standard-Hollinger</b> [10x12x15] (inches)"
        );
        assert_eq!(
            container_profile["title_sort"],
            "Standard Hollinger 10x12x15 inches"
        );
        assert_eq!(
            container_profile["display_string"],
            "<b>Standard-Hollinger</b> [10x12x15] (inches)"
        );
        assert_eq!(container_profile["note"], "Acid-free archival carton");
        assert_eq!(
            container_profile["notes_published"],
            "Schema string notes field"
        );
        assert_eq!(container_profile["notes"], "Schema string notes field");
        assert_eq!(container_profile["container_profile_width_u_sstr"], "10");
        assert_eq!(container_profile["container_profile_height_u_sstr"], "12");
        assert_eq!(container_profile["container_profile_depth_u_sstr"], "15");
        assert_eq!(
            container_profile["container_profile_dimension_units_u_sstr"],
            "inches"
        );
        assert_eq!(
            container_profile["typeahead_sort_key_u_sort"],
            "<b>Standard-Hollinger</b> [10x12x15] (inches)"
        );
        assert_eq!(container_profile["external_id"], json!(["cp-3"]));
        assert_eq!(container_profile["ancestors"], json!([]));
        assert_eq!(container_profile["langcode"], json!(["eng"]));
        assert_eq!(
            container_profile["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "profile_assessment_metadata"
            ])
        );

        let location_profile = build_index_document(IndexDocumentInput {
            uri: "/location_profiles/4".to_string(),
            record_type: "location_profile".to_string(),
            repository: Value::String("global".to_string()),
            record: json!({
                "uri": "/location_profiles/4",
                "name": "Deep Folio Shelf",
                "display_string": "<i>Deep-Folio_Shelf</i> #4 (cm)",
                "width": "120",
                "height": "40",
                "depth": "60",
                "dimension_units": "centimeters",
                "external_ids": [{"external_id": "lp-4"}],
                "ancestors": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(location_profile["title"], "<i>Deep-Folio_Shelf</i> #4 (cm)");
        assert_eq!(location_profile["title_sort"], "Deep Folio_Shelf 4 cm");
        assert_eq!(
            location_profile["display_string"],
            "<i>Deep-Folio_Shelf</i> #4 (cm)"
        );
        assert!(location_profile.get("note").is_none());
        assert_eq!(location_profile["location_profile_width_u_sstr"], "120");
        assert_eq!(location_profile["location_profile_height_u_sstr"], "40");
        assert_eq!(location_profile["location_profile_depth_u_sstr"], "60");
        assert_eq!(
            location_profile["location_profile_dimension_units_u_sstr"],
            "centimeters"
        );
        assert_eq!(
            location_profile["typeahead_sort_key_u_sort"],
            "<i>Deep-Folio_Shelf</i> #4 (cm)"
        );
        assert_eq!(location_profile["external_id"], json!(["lp-4"]));
        assert_eq!(location_profile["ancestors"], json!([]));
        assert_eq!(location_profile["langcode"], json!([]));
        assert_eq!(
            location_profile["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "profile_assessment_metadata"
            ])
        );
    }

    #[test]
    fn builds_assessment_staff_index_fields_for_full_and_minimal_records() {
        let full_assessment = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/assessments/42".to_string(),
            record_type: "assessment".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/assessments/42",
                "display_string": "Assessment 42 — Main Collection",
                "publish": false,
                "records": [
                    {
                        "ref": "/repositories/2/archival_objects/10",
                        "_resolved": {
                            "jsonmodel_type": "archival_object",
                            "display_string": "Series I: Letters",
                            "title": "Letters"
                        }
                    },
                    {
                        "ref": "/repositories/2/resources/7",
                        "_resolved": {
                            "jsonmodel_type": "resource",
                            "display_string": Value::Null,
                            "title": "Sánchez Papers"
                        }
                    },
                    {
                        "ref": "/repositories/2/accessions/3",
                        "_resolved": {
                            "jsonmodel_type": "accession",
                            "title": "2026-003 Accession"
                        }
                    },
                    {
                        "ref": "/repositories/2/archival_objects/11",
                        "_resolved": {
                            "jsonmodel_type": "archival_object",
                            "display_string": "Series II: Diaries"
                        }
                    }
                ],
                "surveyed_by": [
                    {
                        "ref": "/agents/people/1",
                        "_resolved": {"title": "Surveyor One"}
                    },
                    {
                        "ref": "/agents/people/2",
                        "_resolved": {"title": "Surveyor Two"}
                    }
                ],
                "survey_begin": "2024-03-15",
                "survey_end": "2026-04-01",
                "review_required": true,
                "sensitive_material": true,
                "inactive": true,
                "reviewer": [
                    {
                        "ref": "/agents/people/9",
                        "_resolved": {"title": "Reviewer Nine"}
                    }
                ],
                "collections": [
                    {
                        "ref": "/repositories/2/resources/7",
                        "_resolved": {
                            "display_string": "Sánchez Display",
                            "title": "Sánchez Title"
                        }
                    },
                    {
                        "ref": "/repositories/2/resources/8",
                        "_resolved": {
                            "title": "Fallback Collection Title"
                        }
                    }
                ],
                "formats": [
                    {"label": "Architectural Drawings", "value": "true"},
                    {"label": "Audio Materials", "note": "note without value"},
                    {"label": "Photographs"}
                ],
                "ratings": [
                    {"label": "Documentation Quality", "value": "4"},
                    {"label": "Research Value", "note": "High research interest"},
                    {"label": "Housing Quality"}
                ],
                "conservation_issues": [
                    {"label": "Brittle Paper", "value": "true"},
                    {"label": "Mold", "note": "note only must not match"},
                    {"label": "Water Damage"}
                ],
                "external_ids": [{"external_id": "assess-42"}],
                "ancestors": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(full_assessment["assessment_id"], 42);
        assert_eq!(full_assessment["title"], "Assessment 42 — Main Collection");
        assert_eq!(
            full_assessment["display_string"],
            "Assessment 42 — Main Collection"
        );
        assert_eq!(
            full_assessment["assessment_record_uris"],
            json!([
                "/repositories/2/archival_objects/10",
                "/repositories/2/resources/7",
                "/repositories/2/accessions/3",
                "/repositories/2/archival_objects/11"
            ])
        );
        assert_eq!(
            full_assessment["assessment_records"],
            json!([
                "Series I: Letters",
                "Sánchez Papers",
                "2026-003 Accession",
                "Series II: Diaries"
            ])
        );
        assert_eq!(
            full_assessment["assessment_record_types"],
            json!(["accession", "archival_object", "resource"])
        );
        assert_eq!(
            full_assessment["assessment_surveyor_uris"],
            json!(["/agents/people/1", "/agents/people/2"])
        );
        assert_eq!(
            full_assessment["assessment_surveyors"],
            json!(["Surveyor One", "Surveyor Two"])
        );
        assert_eq!(
            full_assessment["assessment_survey_begin"],
            "2024-03-15T00:00:00Z"
        );
        assert_eq!(
            full_assessment["assessment_survey_end"],
            "2026-04-01T00:00:00Z"
        );
        assert_eq!(full_assessment["assessment_review_required"], true);
        assert_eq!(full_assessment["assessment_sensitive_material"], true);
        assert_eq!(full_assessment["assessment_inactive"], true);
        assert_eq!(
            full_assessment["assessment_reviewer_uris"],
            json!(["/agents/people/9"])
        );
        assert_eq!(
            full_assessment["assessment_reviewers"],
            json!(["Reviewer Nine"])
        );
        assert_eq!(
            full_assessment["assessment_survey_year"],
            json!(["2024", "2025", "2026"])
        );
        assert_eq!(
            full_assessment["assessment_collection_uris"],
            json!(["/repositories/2/resources/7", "/repositories/2/resources/8"])
        );
        assert_eq!(
            full_assessment["assessment_collections"],
            json!(["Sánchez Display", "Fallback Collection Title"])
        );
        assert_eq!(full_assessment["assessment_completed"], true);
        assert_eq!(
            full_assessment["assessment_formats"],
            json!(["Architectural Drawings"])
        );
        assert_eq!(
            full_assessment["assessment_ratings"],
            json!(["Documentation Quality", "Research Value"])
        );
        assert_eq!(
            full_assessment["assessment_conservation_issues"],
            json!(["Brittle Paper"])
        );
        assert_eq!(full_assessment["title_sort"], "0000000042");
        assert_eq!(full_assessment["external_id"], json!(["assess-42"]));
        assert_eq!(
            full_assessment["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "profile_assessment_metadata"
            ])
        );

        let minimal_assessment = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/assessments/7".to_string(),
            record_type: "assessment".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/assessments/7",
                "display_string": "Minimal Assessment",
                "records": [{
                    "ref": "/repositories/2/resources/1",
                    "_resolved": {
                        "jsonmodel_type": "resource",
                        "title": "Resource One"
                    }
                }],
                "surveyed_by": [{
                    "ref": "/agents/people/1",
                    "_resolved": {"title": "Surveyor One"}
                }],
                "survey_begin": "2025-11-01",
                "survey_end": Value::Null,
                "review_required": Value::Null,
                "sensitive_material": false,
                "reviewer": [],
                "collections": [],
                "formats": [],
                "ratings": [],
                "conservation_issues": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(minimal_assessment["assessment_id"], 7);
        assert_eq!(minimal_assessment["title_sort"], "0000000007");
        assert_eq!(
            minimal_assessment["assessment_survey_begin"],
            "2025-11-01T00:00:00Z"
        );
        assert!(minimal_assessment.get("assessment_survey_end").is_none());
        assert_eq!(minimal_assessment["assessment_review_required"], false);
        assert_eq!(minimal_assessment["assessment_sensitive_material"], false);
        assert_eq!(minimal_assessment["assessment_inactive"], false);
        assert!(minimal_assessment.get("assessment_reviewer_uris").is_none());
        assert!(minimal_assessment.get("assessment_reviewers").is_none());
        assert_eq!(
            minimal_assessment["assessment_survey_year"],
            json!(["2025"])
        );
        assert_eq!(minimal_assessment["assessment_completed"], false);
        assert_eq!(minimal_assessment["assessment_formats"], json!([]));
        assert_eq!(minimal_assessment["assessment_ratings"], json!([]));
        assert_eq!(
            minimal_assessment["assessment_conservation_issues"],
            json!([])
        );
    }

    #[test]
    fn matches_native_generate_years_for_date_range_edge_cases() {
        assert_eq!(
            generate_years_for_date_range(None, Some(&json!("2025-01-01"))),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_years_for_date_range(Some(&Value::Null), Some(&json!("2025-01-01"))),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("2025-01-01")), None),
            vec![json!("2025")]
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("2025-01-01")), Some(&Value::Null)),
            vec![json!("2025")]
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("2026-01-01")), Some(&json!("2024-01-01"))),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("2025-01-01")), Some(&json!("unknown"))),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("unknown")), Some(&json!("2025-01-01"))),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("9")), Some(&json!("11"))),
            vec![json!("9"), json!("10"), json!("11")]
        );
        assert_eq!(
            generate_years_for_date_range(Some(&json!("09")), Some(&json!("11"))),
            vec![json!("09"), json!("10"), json!("11")]
        );
    }

    #[test]
    fn builds_top_container_staff_index_fields_for_full_and_minimal_records() {
        let full_top_container = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/55".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/55",
                "title": "Envelope Title",
                "display_string": "Box 12-A: Sánchez & Co. (Éire) #3",
                "long_display_string": "Collection MS-7, Series I, Box 12-A [BC-55]",
                "type": "box",
                "indicator": "12-A",
                "barcode": "BC-00055",
                "ils_holding_id": "ILS-HOLD-55",
                "subcontainer_barcodes": "SUB-BC-1, SUB-BC-2",
                "created_for_collection": "/repositories/2/resources/7",
                "internal_note": "Fragile clasp; keep flat",
                "exported_to_ils": "2026-02-01T12:00:00Z",
                "is_linked_to_published_record": true,
                "series": [
                    {
                        "ref": "/repositories/2/archival_objects/101",
                        "display_string": "Series I: Correspondence",
                        "level_display_string": "Series",
                        "identifier": "MS-123.45/a_b",
                        "publish": true
                    },
                    {
                        "ref": "/repositories/2/archival_objects/102",
                        "display_string": "Series II: Restricted Drafts",
                        "level_display_string": "Subseries",
                        "identifier": "MS 123",
                        "publish": false
                    },
                    {
                        "ref": "/repositories/2/archival_objects/103",
                        "display_string": "Series III: Photographs",
                        "level_display_string": "Series",
                        "identifier": "123",
                        "publish": true
                    },
                    {
                        "ref": "/repositories/2/archival_objects/104",
                        "display_string": "Series IV: Unnumbered",
                        "level_display_string": "Series",
                        "identifier": Value::Null
                    }
                ],
                "collection": [
                    {
                        "ref": "/repositories/2/resources/7",
                        "display_string": "Sánchez Papers",
                        "identifier": "A - 12..34_B"
                    },
                    {
                        "ref": "/repositories/2/accessions/9",
                        "display_string": "2026-009 Accession",
                        "identifier": "2026–009"
                    }
                ],
                "container_profile": {
                    "ref": "/container_profiles/3",
                    "_resolved": {
                        "display_string": "Standard Hollinger [10x12x15]"
                    }
                },
                "container_locations": [
                    {
                        "status": "previous",
                        "ref": "/locations/10",
                        "_resolved": {
                            "title": "Old Annex, Shelf 1",
                            "location_profile": {
                                "ref": "/location_profiles/1",
                                "_resolved": {"display_string": "Old Shelf Profile"}
                            }
                        }
                    },
                    {
                        "status": "current",
                        "ref": "/locations/20",
                        "_resolved": {
                            "title": "Main Vault, Bay 1",
                            "location_profile": {
                                "ref": "/location_profiles/4",
                                "_resolved": {"display_string": "Standard Folio Shelf"}
                            }
                        }
                    },
                    {
                        "status": "current",
                        "ref": "/locations/20",
                        "_resolved": {
                            "title": "Main Vault, Bay 1",
                            "location_profile": {
                                "ref": "/location_profiles/4",
                                "_resolved": {"display_string": "Standard Folio Shelf"}
                            }
                        }
                    },
                    {
                        "status": "current",
                        "ref": "/locations/21",
                        "_resolved": {
                            "title": "Main Vault, Bay 2",
                            "location_profile": {
                                "ref": "/location_profiles/5",
                                "_resolved": {"display_string": "Deep Oversize Shelf"}
                            }
                        }
                    },
                    {
                        "status": "current",
                        "ref": "/locations/22",
                        "_resolved": {
                            "title": "Main Vault, Bay 3"
                        }
                    }
                ],
                "external_ids": [{"external_id": "tc-ext-55"}],
                "ancestors": [],
                "lang_materials": [{"language_and_script": {"language": "eng"}}]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(
            full_top_container["title"],
            "Collection MS-7, Series I, Box 12-A [BC-55]"
        );
        assert_eq!(
            full_top_container["display_string"],
            "Box 12-A: Sánchez & Co. (Éire) #3"
        );
        assert_eq!(full_top_container["type_u_ssort"], "box");
        assert_eq!(full_top_container["notes"], "Fragile clasp; keep flat");
        assert_eq!(
            full_top_container["series_uri_u_sstr"],
            json!([
                "/repositories/2/archival_objects/101",
                "/repositories/2/archival_objects/102",
                "/repositories/2/archival_objects/103",
                "/repositories/2/archival_objects/104"
            ])
        );
        assert_eq!(
            full_top_container["series_title_u_sstr"],
            json!([
                "Series I: Correspondence",
                "Series II: Restricted Drafts",
                "Series III: Photographs",
                "Series IV: Unnumbered"
            ])
        );
        assert_eq!(
            full_top_container["series_level_u_sstr"],
            json!(["Series", "Subseries", "Series", "Series"])
        );
        assert_eq!(
            full_top_container["series_identifier_stored_u_sstr"],
            json!(["MS-123.45/a_b", "MS 123", "123", Value::Null])
        );
        assert_eq!(
            full_top_container["series_identifier_u_stext"],
            json!([
                "MS-123.45/a_b",
                "MS 123 45 a b",
                "MS12345ab",
                "MS- 123 . 45 /a_b",
                "MS 123",
                "MS123",
                "MS  123",
                "123"
            ])
        );
        assert_eq!(
            full_top_container["published_series_uri_u_sstr"],
            json!([
                "/repositories/2/archival_objects/101",
                "/repositories/2/archival_objects/103"
            ])
        );
        assert_eq!(
            full_top_container["published_series_title_u_sstr"],
            json!(["Series I: Correspondence", "Series III: Photographs"])
        );
        assert_eq!(
            full_top_container["collection_uri_u_sstr"],
            json!([
                "/repositories/2/resources/7",
                "/repositories/2/accessions/9"
            ])
        );
        assert_eq!(
            full_top_container["collection_display_string_u_sstr"],
            json!(["Sánchez Papers", "2026-009 Accession"])
        );
        assert_eq!(
            full_top_container["collection_identifier_stored_u_sstr"],
            json!(["A - 12..34_B", "2026–009"])
        );
        assert_eq!(
            full_top_container["collection_identifier_u_stext"],
            json!([
                "A - 12..34_B",
                "A   12 34 B",
                "A1234B",
                "A -  12 .. 34 _B",
                "2026–009",
                "2026 009",
                "2026009",
                "2026 – 009"
            ])
        );
        assert_eq!(
            full_top_container["container_profile_uri_u_sstr"],
            "/container_profiles/3"
        );
        assert_eq!(
            full_top_container["container_profile_display_string_u_sstr"],
            "Standard Hollinger [10x12x15]"
        );
        assert_eq!(full_top_container["has_location_u_sbool"], true);
        assert_eq!(
            full_top_container["location_uri_u_sstr"],
            json!([
                "/locations/20",
                "/locations/20",
                "/locations/21",
                "/locations/22"
            ])
        );
        assert_eq!(
            full_top_container["location_uris"],
            json!([
                "/locations/20",
                "/locations/20",
                "/locations/21",
                "/locations/22"
            ])
        );
        assert_eq!(
            full_top_container["location_display_string_u_sstr"],
            json!([
                "Main Vault, Bay 1",
                "Main Vault, Bay 1",
                "Main Vault, Bay 2",
                "Main Vault, Bay 3"
            ])
        );
        assert_eq!(
            full_top_container["location_profile_display_string_u_sstr"],
            json!(["Standard Folio Shelf", "Deep Oversize Shelf"])
        );
        assert_eq!(full_top_container["exported_u_sbool"], true);
        assert_eq!(full_top_container["empty_u_sbool"], false);
        assert_eq!(full_top_container["indicator_u_icusort"], "12-A");
        assert_eq!(
            full_top_container["top_container_u_typeahead_utext"],
            "box12asnchezcoire3"
        );
        assert_eq!(
            full_top_container["top_container_u_icusort"],
            "Box 12-A: Sánchez & Co. (Éire) #3"
        );
        assert_eq!(full_top_container["barcode_u_sstr"], "BC-00055");
        assert_eq!(full_top_container["barcode_u_icusort"], "BC-00055");
        assert_eq!(full_top_container["ils_holding_id_u_sstr"], "ILS-HOLD-55");
        assert_eq!(
            full_top_container["subcontainer_barcodes_u_sstr"],
            "SUB-BC-1, SUB-BC-2"
        );
        assert_eq!(
            full_top_container["created_for_collection_u_sstr"],
            "/repositories/2/resources/7"
        );
        assert_eq!(full_top_container["external_id"], json!(["tc-ext-55"]));
        assert_eq!(full_top_container["ancestors"], json!([]));
        assert_eq!(full_top_container["langcode"], json!(["eng"]));
        assert_eq!(full_top_container["types"], json!(["top_container"]));
        assert_eq!(full_top_container["publish"], false);
        assert_eq!(
            full_top_container["_archivalist_rust_fields"],
            json!(["common_metadata", "fullrecord", "top_container_metadata"])
        );

        let minimal_top_container = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/56".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/56",
                "display_string": "Folder 1",
                "collection": [],
                "container_locations": []
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(minimal_top_container["title"], Value::Null);
        assert_eq!(minimal_top_container["display_string"], "Folder 1");
        assert_eq!(minimal_top_container["type_u_ssort"], Value::Null);
        assert_eq!(minimal_top_container["notes"], Value::Null);
        assert!(minimal_top_container.get("series_uri_u_sstr").is_none());
        assert!(minimal_top_container.get("series_title_u_sstr").is_none());
        assert!(minimal_top_container.get("series_level_u_sstr").is_none());
        assert!(minimal_top_container
            .get("series_identifier_stored_u_sstr")
            .is_none());
        assert!(minimal_top_container
            .get("series_identifier_u_stext")
            .is_none());
        assert!(minimal_top_container
            .get("published_series_uri_u_sstr")
            .is_none());
        assert!(minimal_top_container
            .get("published_series_title_u_sstr")
            .is_none());
        assert_eq!(minimal_top_container["collection_uri_u_sstr"], json!([]));
        assert_eq!(
            minimal_top_container["collection_display_string_u_sstr"],
            json!([])
        );
        assert_eq!(
            minimal_top_container["collection_identifier_stored_u_sstr"],
            json!([])
        );
        assert_eq!(
            minimal_top_container["collection_identifier_u_stext"],
            json!([])
        );
        assert!(minimal_top_container
            .get("container_profile_uri_u_sstr")
            .is_none());
        assert!(minimal_top_container
            .get("container_profile_display_string_u_sstr")
            .is_none());
        assert_eq!(minimal_top_container["has_location_u_sbool"], false);
        assert!(minimal_top_container.get("location_uri_u_sstr").is_none());
        assert!(minimal_top_container.get("location_uris").is_none());
        assert!(minimal_top_container
            .get("location_display_string_u_sstr")
            .is_none());
        assert!(minimal_top_container
            .get("location_profile_display_string_u_sstr")
            .is_none());
        assert_eq!(minimal_top_container["exported_u_sbool"], false);
        assert_eq!(minimal_top_container["empty_u_sbool"], true);
        assert!(minimal_top_container.get("indicator_u_icusort").is_none());
        assert_eq!(
            minimal_top_container["top_container_u_typeahead_utext"],
            "folder1"
        );
        assert_eq!(minimal_top_container["top_container_u_icusort"], "Folder 1");
        assert_eq!(minimal_top_container["barcode_u_sstr"], Value::Null);
        assert_eq!(minimal_top_container["barcode_u_icusort"], Value::Null);
        assert_eq!(minimal_top_container["ils_holding_id_u_sstr"], Value::Null);
        assert_eq!(
            minimal_top_container["subcontainer_barcodes_u_sstr"],
            Value::Null
        );
        assert_eq!(
            minimal_top_container["created_for_collection_u_sstr"],
            Value::Null
        );
        assert_eq!(
            minimal_top_container["_archivalist_rust_fields"],
            json!(["common_metadata", "fullrecord", "top_container_metadata"])
        );
    }

    #[test]
    fn matches_top_container_edge_cases_for_series_locations_exported_and_identifiers() {
        let unpublished_and_previous_only = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/57".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/57",
                "display_string": "Carton 99",
                "indicator": false,
                "exported_to_ils": false,
                "series": [
                    {
                        "ref": "/repositories/2/archival_objects/201",
                        "display_string": "Unpublished Series One",
                        "level_display_string": "Series",
                        "identifier": "S1",
                        "publish": false
                    },
                    {
                        "ref": "/repositories/2/archival_objects/202",
                        "display_string": "Unpublished Series Two",
                        "level_display_string": "Series",
                        "identifier": "S1",
                        "publish": Value::Null
                    }
                ],
                "container_locations": [
                    {
                        "status": "previous",
                        "ref": "/locations/99",
                        "_resolved": {
                            "title": "Former Staging Room"
                        }
                    }
                ]
            }),
            enrich: true,
        })
        .unwrap();

        assert_eq!(
            unpublished_and_previous_only["series_identifier_u_stext"],
            json!(["S1", "S 1", "S1", "S 1"])
        );
        assert!(unpublished_and_previous_only
            .get("published_series_uri_u_sstr")
            .is_none());
        assert!(unpublished_and_previous_only
            .get("published_series_title_u_sstr")
            .is_none());
        assert!(unpublished_and_previous_only
            .get("collection_uri_u_sstr")
            .is_none());
        assert_eq!(unpublished_and_previous_only["empty_u_sbool"], true);
        assert_eq!(unpublished_and_previous_only["exported_u_sbool"], true);
        assert!(unpublished_and_previous_only
            .get("indicator_u_icusort")
            .is_none());
        assert_eq!(unpublished_and_previous_only["has_location_u_sbool"], true);
        assert_eq!(
            unpublished_and_previous_only["location_uri_u_sstr"],
            json!([])
        );
        assert_eq!(unpublished_and_previous_only["location_uris"], json!([]));
        assert_eq!(
            unpublished_and_previous_only["location_display_string_u_sstr"],
            json!([])
        );
        assert_eq!(
            unpublished_and_previous_only["location_profile_display_string_u_sstr"],
            json!([])
        );

        let exported_null = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/58".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/58",
                "display_string": "Box 58",
                "indicator": Value::Null,
                "exported_to_ils": Value::Null
            }),
            enrich: true,
        })
        .unwrap();
        assert_eq!(exported_null["exported_u_sbool"], true);
        assert!(exported_null.get("indicator_u_icusort").is_none());

        assert_eq!(
            generate_permutations_for_identifier(None, "id").unwrap(),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_permutations_for_identifier(Some(&Value::Null), "id").unwrap(),
            Vec::<Value>::new()
        );
        assert_eq!(
            generate_permutations_for_identifier(Some(&json!("123")), "id").unwrap(),
            vec![json!("123")]
        );
        assert_eq!(
            generate_permutations_for_identifier(Some(&json!("ABC")), "id").unwrap(),
            vec![json!("ABC")]
        );
        assert_eq!(
            generate_permutations_for_identifier(Some(&json!("---")), "id").unwrap(),
            vec![json!("---"), json!(" "), json!("")]
        );
    }

    #[test]
    fn keeps_unmigrated_types_and_unenriched_requests_free_of_type_specific_markers() {
        let job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/5".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/5",
                "title": "Job 5"
            }),
            enrich: true,
        })
        .unwrap();
        assert_eq!(
            job["_archivalist_rust_fields"],
            json!(["common_metadata", "fullrecord"])
        );

        let unenriched_top_container = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/55".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/55",
                "title": "Envelope title",
                "display_string": "Box 55",
                "long_display_string": "Long Box 55"
            }),
            enrich: false,
        })
        .unwrap();
        assert_eq!(unenriched_top_container["title"], "Envelope title");
        assert!(unenriched_top_container
            .get("_archivalist_rust_fields")
            .is_none());
        assert!(unenriched_top_container
            .get("top_container_u_icusort")
            .is_none());

        let unenriched_assessment = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/assessments/42".to_string(),
            record_type: "assessment".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/assessments/42",
                "title": "Envelope title",
                "display_string": "Display title"
            }),
            enrich: false,
        })
        .unwrap();
        assert_eq!(unenriched_assessment["title"], "Envelope title");
        assert!(unenriched_assessment
            .get("_archivalist_rust_fields")
            .is_none());
        assert!(unenriched_assessment.get("assessment_id").is_none());
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
        for bad_uri in [
            "/assessments/42",
            "/repositories//assessments/42",
            "/repositories/2/assessments/not-a-number",
            "/repositories/2/resources/42",
        ] {
            assert!(build_index_document(IndexDocumentInput {
                uri: bad_uri.to_string(),
                record_type: "assessment".to_string(),
                repository: json!(2),
                record: json!({
                    "uri": bad_uri,
                    "display_string": "Bad Assessment"
                }),
                enrich: true,
            })
            .is_err());
        }
        assert!(build_index_document(IndexDocumentInput {
            uri: "/repositories/2/assessments/42".to_string(),
            record_type: "assessment".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/assessments/42",
                "display_string": "Bad Formats Assessment",
                "formats": "not-an-array"
            }),
            enrich: true,
        })
        .is_err());
        assert!(build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/55".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/55"
            }),
            enrich: true,
        })
        .is_err());
        assert!(build_index_document(IndexDocumentInput {
            uri: "/repositories/2/top_containers/55".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/55",
                "display_string": "Box 55",
                "container_locations": [{"status": "current", "ref": "/locations/1"}]
            }),
            enrich: true,
        })
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
