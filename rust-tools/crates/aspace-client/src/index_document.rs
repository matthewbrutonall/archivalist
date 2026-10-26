use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub const MAX_INDEX_DOCUMENT_BATCH: usize = 1_000;
pub const PUI_FINAL_DOCUMENT_METADATA_MARKER: &str = "pui_final_document_metadata";
pub const PUI_FINAL_DOCUMENTS_KEY: &str = "_archivalist_pui_final";
pub const PUI_ROUTING_METADATA_MARKER: &str = "pui_routing_metadata";
pub const PUI_ROUTING_DOCUMENTS_KEY: &str = "_archivalist_pui_routing";
pub const PUI_NOTES_METADATA_MARKER: &str = "pui_notes_metadata";
pub const PUI_NOTES_DOCUMENTS_KEY: &str = "_archivalist_pui_notes";
pub const PUI_AVAILABILITY_METADATA_MARKER: &str = "pui_availability_metadata";
pub const PUI_AVAILABILITY_DOCUMENTS_KEY: &str = "_archivalist_pui_availability";
pub const PUI_AUXILIARY_METADATA_MARKER: &str = "pui_auxiliary_document_metadata";
pub const PUI_AUXILIARY_DOCUMENTS_KEY: &str = "_archivalist_pui_auxiliary";
pub const PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER: &str =
    "pui_record_inheritance_candidate_metadata";
pub const PUI_CANDIDATE_DOCUMENTS_KEY: &str = "_archivalist_pui_candidate";
pub const PUI_COMMON_METADATA_MARKER: &str = "pui_common_metadata";
pub const PUI_COMMON_DOCUMENTS_KEY: &str = "_archivalist_pui_common";
pub const PUI_COMMON_FIELDS: &[&str] = &[
    "marker",
    "uri",
    "primary_type",
    "subjects",
    "agents",
    "audit_info",
    "years",
    "level",
    "summary",
    "extents",
];
pub const PUI_AUXILIARY_TYPES: &[&str] = &[
    "resource_ordered_records",
    "tree_root",
    "tree_waypoint",
    "tree_node",
    "tree_node_from_root",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecordInheritancePlan {
    pub record_type: String,
    pub rules: Vec<RecordInheritanceRulePlan>,
    #[serde(default)]
    pub composite_identifiers: Option<CompositeIdentifierPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecordInheritanceRulePlan {
    pub property: String,
    pub inherit_directly: bool,
    pub ancestor_index: usize,
    #[serde(default)]
    pub ancestor_element_indices: Option<Vec<usize>>,
    pub inherited_value: Value,
    pub inherited_props: InheritanceProps,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InheritanceProps {
    #[serde(rename = "ref")]
    pub ref_val: Value,
    pub level: Value,
    pub direct: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompositeIdentifierPlan {
    pub pieces: Vec<String>,
    pub delimiter: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct IndexDocumentInput {
    pub uri: String,
    pub record_type: String,
    pub repository: Value,
    pub record: Value,
    #[serde(default)]
    pub enrich: bool,
    #[serde(default)]
    pub dynamic_enum_fields: Vec<String>,
    #[serde(default)]
    pub arks_enabled: bool,
    #[serde(default, alias = "accession_date_unknown")]
    pub accession_date_unknown_label: Option<String>,
    #[serde(default)]
    pub parsed_accession_date_year: Option<Value>,
    #[serde(default)]
    pub prepare_pui_final: bool,
    #[serde(default)]
    pub pui_candidate_record: Option<Value>,
    #[serde(default)]
    pub record_inheritance_plan: Option<RecordInheritancePlan>,
    #[serde(default)]
    pub prepare_pui_availability: bool,
    #[serde(default)]
    pub record_inheritance_has_type: Option<bool>,
    #[serde(default)]
    pub prepare_pui_auxiliary: bool,
    #[serde(default)]
    pub pui_parent_id: Option<String>,
    #[serde(default)]
    pub pui_auxiliary_json: Option<String>,
    #[serde(default)]
    pub prepare_pui_common_metadata: bool,
}

impl Default for IndexDocumentInput {
    fn default() -> Self {
        Self {
            uri: String::new(),
            record_type: String::new(),
            repository: Value::Null,
            record: Value::Null,
            enrich: false,
            dynamic_enum_fields: Vec::new(),
            arks_enabled: false,
            accession_date_unknown_label: None,
            parsed_accession_date_year: None,
            prepare_pui_final: false,
            pui_candidate_record: None,
            record_inheritance_plan: None,
            prepare_pui_availability: false,
            record_inheritance_has_type: None,
            prepare_pui_auxiliary: false,
            pui_parent_id: None,
            pui_auxiliary_json: None,
            prepare_pui_common_metadata: false,
        }
    }
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
    if dates.is_null() || dates == &Value::Bool(false) {
        return Ok(());
    }
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
    } else {
        document.insert("years".to_string(), Value::Array(Vec::new()));
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
            let raw_str = format!(
                "{} {} {}",
                ruby_string(sub_container.get("type_2")),
                ruby_string(sub_container.get("indicator_2")),
                ruby_string(sub_container.get("barcode_2"))
            );
            child_containers.push(Value::String(ruby_strip(&raw_str).to_string()));
        }
        if sub_container
            .get("type_3")
            .is_some_and(|value| !value.is_null())
        {
            let raw_str = format!(
                "{} {}",
                ruby_string(sub_container.get("type_3")),
                ruby_string(sub_container.get("indicator_3"))
            );
            grandchild_containers.push(Value::String(ruby_strip(&raw_str).to_string()));
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
    ruby_clean_for_sort(value)
}

/// Port of IndexerCommon#index_records post-hook title_sort assignment:
///   if ( !values["finding_aid_filing_title"].nil? && values["finding_aid_filing_title"].length > 0 )
///     doc['title_sort'] ||= clean_for_sort(values["finding_aid_filing_title"])
///   else
///     doc['title_sort'] ||= clean_for_sort(values['title'])
///   end
fn apply_title_sort(document: &mut Map<String, Value>, record: &Map<String, Value>) {
    if ruby_truthy(document.get("title_sort")) {
        return;
    }
    let filing_title = record.get("finding_aid_filing_title");
    let candidate = match filing_title {
        Some(Value::String(s)) if !s.is_empty() => Some(s.as_str()),
        _ => match record.get("title") {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => None,
        },
    };
    let title_sort = match candidate {
        Some(s) => Value::String(ruby_clean_for_sort(s)),
        None => Value::Null,
    };
    document.insert("title_sort".to_string(), title_sort);
}

/// Private handoff key carrying synthetic `collection_management` child documents.
/// Ruby removes it before any hook, plugin, or Solr serialization sees the document.
pub const COLLECTION_MANAGEMENT_DOCUMENTS_KEY: &str =
    "_archivalist_collection_management_documents";

fn is_ruby_strip_char(character: char) -> bool {
    matches!(
        character,
        '\0' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' '
    )
}

fn is_ruby_space_class(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ')
}

/// Ruby `String#strip`: ASCII whitespace and NUL at both ends only.
fn ruby_strip(value: &str) -> &str {
    value.trim_matches(is_ruby_strip_char)
}

/// Alphabetic characters for which Rust's `is_alphabetic` is known to equal
/// Ruby's `\p{L}`. Outside these ranges some alphabetic characters are marks or
/// letter numbers, which Ruby's `clean_for_sort` removes.
fn has_exact_sort_letters(value: &str) -> bool {
    value.chars().all(|character| {
        !character.is_alphabetic()
            || character.is_ascii()
            || ('\u{00aa}'..='\u{024f}').contains(&character)
            || ('\u{1e00}'..='\u{1eff}').contains(&character)
    })
}

/// Exact port of `IndexerCommon#clean_for_sort` for inputs accepted by
/// `has_exact_sort_letters`; `\s` and `strip` are ASCII-only in Ruby.
fn ruby_clean_for_sort(value: &str) -> String {
    let without_tags = strip_angle_tags(value);
    let mut output = String::with_capacity(without_tags.len());
    let mut pending_space = false;
    for character in without_tags.chars() {
        let character = if character == '-' { ' ' } else { character };
        if is_ruby_space_class(character) {
            pending_space = true;
        } else if character.is_alphabetic() || character.is_ascii_digit() || character == '_' {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            pending_space = false;
            output.push(character);
        }
    }
    output
}

fn strip_fullrecord_array(values: &mut [Value]) {
    for value in values {
        match value {
            Value::String(text) => {
                let stripped = ruby_strip(text);
                if stripped.len() != text.len() {
                    *text = stripped.to_string();
                }
            }
            Value::Object(object) => strip_fullrecord_object(object),
            Value::Array(nested) => strip_fullrecord_array(nested),
            _ => {}
        }
    }
}

/// Native indexing aliases every `extract_string_values` string into the parent
/// document's `fullrecord`, and `clean_whitespace` then strips those strings in
/// place before the collection-management child hook runs. Mirror that effect.
fn strip_fullrecord_object(object: &mut Map<String, Value>) {
    for (key, value) in object.iter_mut() {
        if FULLRECORD_EXCLUDES.contains(&key.as_str()) || key.ends_with("_enum_s") {
            continue;
        }
        match value {
            Value::String(text) => {
                let stripped = ruby_strip(text);
                if stripped.len() != text.len() {
                    *text = stripped.to_string();
                }
            }
            Value::Object(nested) => strip_fullrecord_object(nested),
            Value::Array(values) => strip_fullrecord_array(values),
            _ => {}
        }
    }
}

/// Builds the synthetic STAFF `collection_management` child document emitted by
/// the native `add_extra_documents_hook`. Returns `None` when the record has no
/// collection-management object, or when exact parity cannot be guaranteed, so
/// Ruby falls back to the native generator for that record.
fn collection_management_document(
    uri: &str,
    record_type: &str,
    repository: &Value,
    record: &Map<String, Value>,
) -> Result<Option<Value>, IndexDocumentError> {
    if !matches!(record_type, "resource" | "accession" | "digital_object") {
        return Ok(None);
    }
    let Some(collection_management) = record
        .get("collection_management")
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    let Some(cm_uri) = collection_management.get("uri").and_then(Value::as_str) else {
        return Ok(None);
    };
    if cm_uri.trim().is_empty() {
        return Ok(None);
    }
    let mut collection_management = collection_management.clone();
    strip_fullrecord_object(&mut collection_management);

    let title = if ruby_truthy(record.get("title")) {
        record.get("title")
    } else {
        record.get("display_string")
    };
    let (title, title_sort) = match title {
        None | Some(Value::Null) => (Value::Null, Value::Null),
        Some(Value::String(text)) => {
            let stripped = ruby_strip(text);
            if !has_exact_sort_letters(stripped) {
                return Ok(None);
            }
            (
                Value::String(stripped.to_string()),
                Value::String(ruby_clean_for_sort(stripped)),
            )
        }
        Some(_) => {
            return Err(error(
                "collection management parent title must be a string or null",
            ))
        }
    };
    let field = |name: &str| {
        collection_management
            .get(name)
            .cloned()
            .unwrap_or(Value::Null)
    };
    let json = serde_json::to_string(&collection_management)
        .map_err(|err| error(format!("failed to serialize collection management: {err}")))?;

    let mut document = Map::new();
    document.insert("id".to_string(), field("uri"));
    document.insert("uri".to_string(), field("uri"));
    document.insert("parent_id".to_string(), json_string(uri));
    document.insert("parent_title".to_string(), title.clone());
    document.insert("parent_type".to_string(), json_string(record_type));
    document.insert("title".to_string(), title);
    document.insert("title_sort".to_string(), title_sort);
    document.insert(
        "types".to_string(),
        Value::Array(vec![json_string("collection_management")]),
    );
    document.insert(
        "primary_type".to_string(),
        json_string("collection_management"),
    );
    document.insert("json".to_string(), Value::String(json));
    for name in [
        "processing_priority",
        "processing_status",
        "processing_hours_total",
        "processing_funding_source",
        "processors",
    ] {
        document.insert(name.to_string(), field(name));
    }
    document.insert(
        "suppressed".to_string(),
        record.get("suppressed").cloned().unwrap_or(Value::Null),
    );
    document.insert("repository".to_string(), repository.clone());
    for name in [
        "created_by",
        "last_modified_by",
        "system_mtime",
        "user_mtime",
        "create_time",
    ] {
        document.insert(name.to_string(), field(name));
    }
    Ok(Some(Value::Object(document)))
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

fn add_job_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    // `common/schemas/job.rb` defines `job` as `"type" => "object"`, while
    // `Job.sequel_to_jsonmodel` (`backend/app/model/job.rb`) falls back to setting
    // `json.job = obj.job_type` (a string) on `JSONModel::ModelNotFound`.
    let report_type = match record.get("job") {
        Some(Value::Object(job)) => job.get("report_type").cloned().unwrap_or(Value::Null),
        Some(Value::String(job)) => {
            if job.contains("report_type") {
                Value::String("report_type".to_string())
            } else {
                Value::Null
            }
        }
        _ => return Err(error("field 'job' must be an object or string")),
    };
    let job_type = record.get("job_type").cloned().unwrap_or(Value::Null);
    let types = document
        .get_mut("types")
        .and_then(Value::as_array_mut)
        .expect("types is initialized as an array");
    types.push(job_type.clone());
    types.push(report_type.clone());
    let job_report_type = if ruby_truthy(Some(&report_type)) {
        report_type.clone()
    } else {
        job_type.clone()
    };
    document.insert("job_type".to_string(), job_type);
    document.insert("report_type".to_string(), report_type);
    document.insert("job_report_type".to_string(), job_report_type);
    for field in ["status", "owner", "queue_position"] {
        document.insert(
            field.to_string(),
            record.get(field).cloned().unwrap_or(Value::Null),
        );
    }
    Ok(())
}

fn add_accession_date_fields(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
    unknown_label: Option<&str>,
    parsed_year_val: Option<&Value>,
    markers: &mut Vec<Value>,
) -> Result<(), IndexDocumentError> {
    let raw_val = record
        .get("accession_date")
        .ok_or_else(|| error("field 'accession_date' is required for accession record"))?;
    let raw_str = match raw_val {
        Value::String(s) => s,
        _ => return Err(error("field 'accession_date' must be a string")),
    };

    let year_value = parsed_year_val.ok_or_else(|| {
        error("field 'parsed_accession_date_year' is required for accession record")
    })?;
    let year_num = match year_value {
        Value::Number(n) => n,
        _ => {
            return Err(error(
                "field 'parsed_accession_date_year' must be an integer",
            ))
        }
    };
    let year_i64 = year_num
        .as_i64()
        .ok_or_else(|| error("field 'parsed_accession_date_year' must be an integer"))?;
    if year_i64 < (i32::MIN as i64) || year_i64 > (i32::MAX as i64) {
        return Err(error(format!(
            "field 'parsed_accession_date_year' {year_i64} is out of valid signed 32-bit integer range"
        )));
    }
    let year = year_i64 as i32;

    if raw_str == "9999-12-31" {
        let label = unknown_label.filter(|l| !l.is_empty()).ok_or_else(|| {
            error(
                "accession_date_unknown_label is required for sentinel accession_date '9999-12-31'",
            )
        })?;
        document.insert(
            "accession_date".to_string(),
            Value::String(label.to_string()),
        );
        document.insert(
            "accession_date_year".to_string(),
            Value::Number(year.into()),
        );
        let fullrecord = document
            .get_mut("fullrecord")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| error("field 'fullrecord' must be an array in document"))?;
        fullrecord.push(Value::String(format!("{label} ")));
    } else {
        document.insert(
            "accession_date".to_string(),
            Value::String(raw_str.to_string()),
        );
        document.insert(
            "accession_date_year".to_string(),
            Value::Number(year.into()),
        );
    }

    markers.push(json_string("accession_date_metadata"));
    Ok(())
}

fn add_type_specific_fields(
    document: &mut Map<String, Value>,
    record_type: &str,
    record: &Map<String, Value>,
    unknown_label: Option<&str>,
    parsed_accession_date_year: Option<&Value>,
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
            add_accession_date_fields(
                document,
                record,
                unknown_label,
                parsed_accession_date_year,
                &mut markers,
            )?;
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
        "job" => {
            add_job_fields(document, record)?;
            markers.push(json_string("job_metadata"));
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
    unknown_label: Option<&str>,
    parsed_accession_date_year: Option<&Value>,
    rust_fields: &mut Vec<Value>,
) -> Result<(), IndexDocumentError> {
    for marker in add_type_specific_fields(
        document,
        record_type,
        record,
        unknown_label,
        parsed_accession_date_year,
    )? {
        if !rust_fields.contains(&marker) {
            rust_fields.push(marker);
        }
    }
    Ok(())
}

fn add_to_set_map(sets: &mut Vec<(String, Vec<Value>)>, key: &str, val: &Value) {
    if let Some((_, set)) = sets.iter_mut().find(|(k, _)| k == key) {
        if !set.contains(val) {
            set.push(val.clone());
        }
    } else {
        sets.push((key.to_string(), vec![val.clone()]));
    }
}

fn search_nested_enums(
    elt: &Value,
    enum_fields: &[String],
    enum_sets: &mut Vec<(String, Vec<Value>)>,
) {
    match elt {
        Value::Object(map) => {
            for key in enum_fields {
                if let Some(field_value) = map.get(key) {
                    let doc_key = format!("{key}_enum_s");
                    add_to_set_map(enum_sets, &doc_key, field_value);
                }
            }
            for (next_key, value) in map {
                if next_key != "_resolved" {
                    search_nested_enums(value, enum_fields, enum_sets);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                search_nested_enums(item, enum_fields, enum_sets);
            }
        }
        _ => {}
    }
}

fn search_nested_items_type(elt: &Value, enum_sets: &mut Vec<(String, Vec<Value>)>) {
    match elt {
        Value::Object(map) => {
            if let Some(Value::Object(items_map)) = map.get("items") {
                if let Some(type_val) = items_map.get("type") {
                    add_to_set_map(enum_sets, "type_enum_s", type_val);
                }
            }
            for (next_key, value) in map {
                if next_key != "_resolved" {
                    search_nested_items_type(value, enum_sets);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                search_nested_items_type(item, enum_sets);
            }
        }
        _ => {}
    }
}

fn flatten_value(val: &Value, out: &mut Vec<Value>) {
    match val {
        Value::Array(items) => {
            for item in items {
                flatten_value(item, out);
            }
        }
        other => {
            out.push(other.clone());
        }
    }
}

fn add_dynamic_enums(
    document: &mut Map<String, Value>,
    full_record: &Value,
    dynamic_enum_fields: &[String],
) {
    let mut enum_sets: Vec<(String, Vec<Value>)> = Vec::new();
    if !dynamic_enum_fields.is_empty() {
        search_nested_enums(full_record, dynamic_enum_fields, &mut enum_sets);
    }
    search_nested_items_type(full_record, &mut enum_sets);

    for (key, set) in enum_sets {
        let mut flattened = Vec::new();
        for item in &set {
            flatten_value(item, &mut flattened);
        }
        document.insert(key, Value::Array(flattened));
    }
}

fn trim_ark_value(s: &str) -> String {
    let (first_line, rest) = match s.split_once('\n') {
        Some((line, tail)) => (line, Some(tail)),
        None => (s, None),
    };
    if let Some(idx) = first_line.rfind("ark:") {
        match rest {
            Some(tail) => format!("ark:{}\n{}", &first_line[idx + 4..], tail),
            None => format!("ark:{}", &first_line[idx + 4..]),
        }
    } else {
        s.to_string()
    }
}

fn add_arks(
    document: &mut Map<String, Value>,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let ark_val = match record.get("ark_name") {
        None | Some(Value::Null) => return Ok(()),
        Some(val) => val,
    };
    let arks = ark_val
        .as_object()
        .ok_or_else(|| error("field 'ark_name' must be an object when present"))?;

    let previous_val = arks
        .get("previous")
        .ok_or_else(|| error("field 'ark_name.previous' is required"))?;
    let previous_arr = previous_val
        .as_array()
        .ok_or_else(|| error("field 'ark_name.previous' must be an array"))?;

    let mut ark_list = Vec::new();

    if let Some(current_val) = arks.get("current") {
        match current_val {
            Value::Null => {}
            Value::String(current_str) => {
                ark_list.push(Value::String(trim_ark_value(current_str)));
            }
            _ => return Err(error("field 'ark_name.current' must be a string or null")),
        }
    }

    for prev_val in previous_arr {
        let prev_str = prev_val
            .as_str()
            .ok_or_else(|| error("elements of 'ark_name.previous' must be strings"))?;
        ark_list.push(Value::String(trim_ark_value(prev_str)));
    }

    document.insert("ark_name".to_string(), Value::Array(ark_list));
    Ok(())
}

fn add_enriched_fields(
    document: &mut Map<String, Value>,
    input: &IndexDocumentInput,
    record: &Map<String, Value>,
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
    let (published, unpublished) = extract_string_values(&input.record);
    document.insert(
        "fullrecord_published".to_string(),
        Value::Array(published.into_iter().map(Value::String).collect()),
    );
    document.insert(
        "fullrecord".to_string(),
        Value::Array(unpublished.into_iter().map(Value::String).collect()),
    );
    let mut rust_fields = vec![json_string("common_metadata"), json_string("fullrecord")];
    add_record_type_markers(
        document,
        &input.record_type,
        record,
        input.accession_date_unknown_label.as_deref(),
        input.parsed_accession_date_year.as_ref(),
        &mut rust_fields,
    )?;
    add_dynamic_enums(document, &input.record, &input.dynamic_enum_fields);
    if input.arks_enabled {
        add_arks(document, record)?;
    }
    rust_fields.push(json_string("dynamic_enum_and_ark_metadata"));
    document.insert(
        "_archivalist_rust_fields".to_string(),
        Value::Array(rust_fields),
    );
    apply_title_sort(document, record);
    Ok(())
}

fn json_string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn clean_pui_candidate_instances(instances: &mut [Value]) {
    for instance in instances.iter_mut() {
        if let Value::Object(instance_obj) = instance {
            if let Some(Value::Object(sub_container)) = instance_obj.get_mut("sub_container") {
                if let Some(Value::Object(top_container)) = sub_container.get_mut("top_container") {
                    if let Some(Value::Object(resolved_top)) = top_container.get_mut("_resolved") {
                        resolved_top.shift_remove("internal_note");
                        if let Some(Value::Object(container_profile)) =
                            resolved_top.get_mut("container_profile")
                        {
                            if let Some(Value::Object(resolved_cp)) =
                                container_profile.get_mut("_resolved")
                            {
                                resolved_cp.shift_remove("notes");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn flatten_json_array(arr: &mut Vec<Value>) {
    let mut flattened = Vec::with_capacity(arr.len());
    for item in arr.drain(..) {
        flatten_json_item(item, &mut flattened);
    }
    *arr = flattened;
}

fn flatten_json_item(item: Value, dest: &mut Vec<Value>) {
    if let Value::Array(sub_arr) = item {
        for sub_item in sub_arr {
            flatten_json_item(sub_item, dest);
        }
    } else {
        dest.push(item);
    }
}

fn execute_record_inheritance_merge(
    record_val: &Value,
    plan: &RecordInheritancePlan,
) -> Result<Map<String, Value>, IndexDocumentError> {
    let mut record = record_val
        .as_object()
        .cloned()
        .ok_or_else(|| error("record must be a JSON object for inheritance merge"))?;

    for rule in &plan.rules {
        if rule.property.trim().is_empty() {
            return Err(error("rule.property must not be empty"));
        }
        if rule.inherited_value.is_null() {
            return Err(error(format!(
                "rule.inherited_value must not be null for property '{}'",
                rule.property
            )));
        }

        let mut props_map = Map::new();
        props_map.insert("ref".to_string(), rule.inherited_props.ref_val.clone());
        props_map.insert("level".to_string(), rule.inherited_props.level.clone());
        props_map.insert(
            "direct".to_string(),
            Value::Bool(rule.inherited_props.direct),
        );
        let props_val = Value::Object(props_map);

        // 1. In-place mutation of the selected ancestor in record["ancestors"]
        let ancestors = record
            .get_mut("ancestors")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                error(format!(
                    "ancestors array missing or not an array for rule on property '{}'",
                    rule.property
                ))
            })?;

        if rule.ancestor_index >= ancestors.len() {
            return Err(error(format!(
                "rule.ancestor_index {} out of bounds (ancestors count {}) for property '{}'",
                rule.ancestor_index,
                ancestors.len(),
                rule.property
            )));
        }

        let ancestor = ancestors[rule.ancestor_index]
            .as_object_mut()
            .ok_or_else(|| {
                error(format!(
                    "ancestor at index {} must be an object",
                    rule.ancestor_index
                ))
            })?;

        let resolved = ancestor
            .get_mut("_resolved")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                error(format!(
                    "ancestor at index {} missing '_resolved' object",
                    rule.ancestor_index
                ))
            })?;

        let ancestor_prop_val = resolved.get_mut(&rule.property);

        if let Some(ref elem_indices) = rule.ancestor_element_indices {
            let ancestor_arr = match ancestor_prop_val {
                Some(Value::Array(arr)) => arr,
                _ => {
                    return Err(error(format!(
                        "ancestor at index {} property '{}' must be an array",
                        rule.ancestor_index, rule.property
                    )));
                }
            };

            for &idx in elem_indices {
                if idx >= ancestor_arr.len() {
                    return Err(error(format!(
                        "ancestor_element_indices index {} out of bounds (length {}) for property '{}'",
                        idx,
                        ancestor_arr.len(),
                        rule.property
                    )));
                }
                if let Value::Object(item_obj) = &mut ancestor_arr[idx] {
                    item_obj.insert("_inherited".to_string(), props_val.clone());
                }
            }
        } else if let Some(Value::Object(item_obj)) = ancestor_prop_val {
            item_obj.insert("_inherited".to_string(), props_val.clone());
        }

        // 2. Destination record update
        let mut val = rule.inherited_value.clone();
        if let Value::Array(items) = &mut val {
            let mut has_non_object = false;
            for item in items.iter_mut() {
                if let Value::Object(item_obj) = item {
                    item_obj.insert("_inherited".to_string(), props_val.clone());
                } else {
                    has_non_object = true;
                }
            }

            let target_arr = match record.get_mut(&rule.property) {
                Some(Value::Array(existing)) => existing,
                _ => {
                    record.insert(rule.property.clone(), Value::Array(Vec::new()));
                    record
                        .get_mut(&rule.property)
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                }
            };
            target_arr.extend(items.clone());
            flatten_json_array(target_arr);

            if has_non_object {
                record.insert(format!("{}_inherited", rule.property), props_val.clone());
            }
        } else if let Value::Object(val_obj) = &mut val {
            val_obj.insert("_inherited".to_string(), props_val);
            record.insert(rule.property.clone(), val);
        } else {
            record.insert(rule.property.clone(), val);
            record.insert(format!("{}_inherited", rule.property), props_val);
        }
    }

    if let Some(ref ci_plan) = plan.composite_identifiers {
        let joined = ci_plan.pieces.join(&ci_plan.delimiter);
        record.insert("_composite_identifier".to_string(), Value::String(joined));
    }

    Ok(record)
}

fn add_pui_final_fields(
    document: &mut Map<String, Value>,
    input: &IndexDocumentInput,
) -> Result<(), IndexDocumentError> {
    // 1. Candidate record: either merged via plan in Rust or passed as pui_candidate_record
    if input.record_inheritance_plan.is_some() && input.pui_candidate_record.is_some() {
        return Err(error(
            "cannot provide both record_inheritance_plan and pui_candidate_record",
        ));
    }

    let (candidate_obj, rust_candidate_map) = if let Some(ref plan) = input.record_inheritance_plan
    {
        if plan.record_type != input.record_type {
            return Err(error(format!(
                "record_inheritance_plan record_type '{}' does not match wrapper record_type '{}'",
                plan.record_type, input.record_type
            )));
        }
        let merged = execute_record_inheritance_merge(&input.record, plan)?;
        let cand_uri = merged
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| error("merged candidate uri must be a string"))?;
        if cand_uri != input.uri {
            return Err(error(format!(
                "merged candidate uri '{cand_uri}' does not match wrapper uri '{}'",
                input.uri
            )));
        }
        (merged.clone(), Some(merged))
    } else {
        let candidate_val = input.pui_candidate_record.as_ref().ok_or_else(|| {
            error("pui_candidate_record is required when prepare_pui_final is true")
        })?;
        let candidate_obj = candidate_val
            .as_object()
            .ok_or_else(|| error("pui_candidate_record must be a JSON object"))?;

        let cand_uri = candidate_obj
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| error("pui_candidate_record.uri must be a string"))?;
        if cand_uri.trim().is_empty() {
            return Err(error("pui_candidate_record.uri must not be empty"));
        }
        if cand_uri != input.uri {
            return Err(error(format!(
                "pui_candidate_record uri '{cand_uri}' does not match wrapper uri '{}'",
                input.uri
            )));
        }

        let cand_type = candidate_obj
            .get("jsonmodel_type")
            .and_then(Value::as_str)
            .ok_or_else(|| error("pui_candidate_record.jsonmodel_type must be a string"))?;
        if cand_type.trim().is_empty() {
            return Err(error(
                "pui_candidate_record.jsonmodel_type must not be empty",
            ));
        }
        if cand_type != input.record_type {
            return Err(error(format!(
                "pui_candidate_record jsonmodel_type '{cand_type}' does not match wrapper record_type '{}'",
                input.record_type
            )));
        }
        (candidate_obj.clone(), None)
    };

    // 2. Strict validation of direct record identity
    let record_obj = input
        .record
        .as_object()
        .ok_or_else(|| error("record must be a JSON object"))?;
    let rec_uri = record_obj
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| error("record.uri must be a string"))?;
    if rec_uri.trim().is_empty() {
        return Err(error("record.uri must not be empty"));
    }
    if rec_uri != input.uri {
        return Err(error(format!(
            "record uri '{rec_uri}' does not match wrapper uri '{}'",
            input.uri
        )));
    }
    let rec_type = record_obj
        .get("jsonmodel_type")
        .and_then(Value::as_str)
        .ok_or_else(|| error("record.jsonmodel_type must be a string"))?;
    if rec_type.trim().is_empty() {
        return Err(error("record.jsonmodel_type must not be empty"));
    }
    if rec_type != input.record_type {
        return Err(error(format!(
            "record jsonmodel_type '{rec_type}' does not match wrapper record_type '{}'",
            input.record_type
        )));
    }

    // 3. Prepare PUI identity / routing metadata projection
    let mut routing_map = Map::new();
    routing_map.insert(
        "marker".to_string(),
        Value::String(PUI_ROUTING_METADATA_MARKER.to_string()),
    );
    routing_map.insert(
        "id".to_string(),
        Value::String(format!("{}#pui", input.uri)),
    );
    routing_map.insert(
        "pui_parent_id".to_string(),
        Value::String(input.uri.clone()),
    );
    routing_map.insert(
        "types".to_string(),
        Value::Array(vec![
            Value::String("pui".to_string()),
            Value::String(format!("pui_{}", input.record_type)),
            Value::String("pui_record".to_string()),
            Value::String("pui_only".to_string()),
        ]),
    );
    document.insert(
        PUI_ROUTING_DOCUMENTS_KEY.to_string(),
        Value::Object(routing_map),
    );

    // 4. Prepare PUI notes_published projection from direct-only record's notes
    let mut notes_map = Map::new();
    notes_map.insert(
        "marker".to_string(),
        Value::String(PUI_NOTES_METADATA_MARKER.to_string()),
    );
    let notes_val = record_obj.get("notes");
    let has_notes = !matches!(
        notes_val,
        None | Some(Value::Null) | Some(Value::Bool(false))
    );
    if has_notes {
        let (published_note_strings, _unpublished) = extract_string_values(notes_val.unwrap());
        notes_map.insert(
            "notes_published".to_string(),
            Value::Array(
                published_note_strings
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
    } else {
        notes_map.insert("notes_published".to_string(), Value::Null);
    }
    document.insert(
        PUI_NOTES_DOCUMENTS_KEY.to_string(),
        Value::Object(notes_map),
    );

    // 5. Prepare final PUI document privacy and fullrecord projection
    let mut clean_candidate = candidate_obj;

    // Remove ancestors from the final PUI JSON candidate
    clean_candidate.shift_remove("ancestors");

    // Remove top-container internal_note and container-profile notes within instances
    if let Some(Value::Array(instances)) = clean_candidate.get_mut("instances") {
        clean_pui_candidate_instances(instances);
    }

    // Serialize privacy-clean fully inherited record into final json
    let pui_json_string = serde_json::to_string(&clean_candidate)
        .map_err(|err| error(format!("failed to serialize clean PUI record: {err}")))?;

    // Generate fullrecord_published using exact native extract_string_values semantics (:published_only)
    let (published_strings, _unpublished) = extract_string_values(&Value::Object(clean_candidate));

    // Capability markers and PUI final payload
    let mut rust_fields = vec![
        Value::String(PUI_FINAL_DOCUMENT_METADATA_MARKER.to_string()),
        Value::String(PUI_ROUTING_METADATA_MARKER.to_string()),
        Value::String(PUI_NOTES_METADATA_MARKER.to_string()),
    ];
    if let Some(ref cand_map) = rust_candidate_map {
        rust_fields.push(Value::String(
            PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER.to_string(),
        ));
        let mut cand_proj = Map::new();
        cand_proj.insert(
            "marker".to_string(),
            Value::String(PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER.to_string()),
        );
        cand_proj.insert("record".to_string(), Value::Object(cand_map.clone()));
        document.insert(
            PUI_CANDIDATE_DOCUMENTS_KEY.to_string(),
            Value::Object(cand_proj),
        );
    }
    document.insert(
        "_archivalist_rust_fields".to_string(),
        Value::Array(rust_fields),
    );

    let mut pui_final_map = Map::new();
    pui_final_map.insert(
        "marker".to_string(),
        Value::String(PUI_FINAL_DOCUMENT_METADATA_MARKER.to_string()),
    );
    pui_final_map.insert("json".to_string(), Value::String(pui_json_string));
    pui_final_map.insert(
        "fullrecord_published".to_string(),
        Value::Array(published_strings.into_iter().map(Value::String).collect()),
    );

    document.insert(
        PUI_FINAL_DOCUMENTS_KEY.to_string(),
        Value::Object(pui_final_map),
    );

    // CRITICAL: Never emit unpublished/private fullrecord for this PUI slice
    document.shift_remove("fullrecord");

    Ok(())
}

pub fn compute_pui_conditional_additions(
    record_type: &str,
    record_inheritance_has_type: bool,
) -> Vec<String> {
    let mut additions = Vec::new();
    if matches!(
        record_type,
        "accession" | "digital_object" | "digital_object_component"
    ) {
        additions.push("pui_record".to_string());
    }
    if matches!(record_type, "agent_person" | "agent_corporate_entity") {
        additions.push("pui_agent".to_string());
    }
    if !record_inheritance_has_type {
        match record_type {
            "resource" => additions.push("pui_collection".to_string()),
            "classification" => additions.push("pui_record_group".to_string()),
            "agent_person" => additions.push("pui_person".to_string()),
            other => additions.push(format!("pui_{other}")),
        }
        additions.push("pui".to_string());
    }
    additions
}

fn add_pui_availability_fields(
    document: &mut Map<String, Value>,
    input: &IndexDocumentInput,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let uri_val = record
        .get("uri")
        .ok_or_else(|| error("record.uri is required when prepare_pui_availability is true"))?;
    let rec_uri = uri_val
        .as_str()
        .ok_or_else(|| error("record.uri must be a non-empty string"))?;
    if rec_uri.trim().is_empty() {
        return Err(error("record.uri must not be empty"));
    }
    if rec_uri != input.uri {
        return Err(error(format!(
            "record uri '{rec_uri}' does not match wrapper uri '{}'",
            input.uri
        )));
    }

    let type_val = record.get("jsonmodel_type").ok_or_else(|| {
        error("record.jsonmodel_type is required when prepare_pui_availability is true")
    })?;
    let rec_type = type_val
        .as_str()
        .ok_or_else(|| error("record.jsonmodel_type must be a non-empty string"))?;
    if rec_type.trim().is_empty() {
        return Err(error("record.jsonmodel_type must not be empty"));
    }
    if rec_type != input.record_type {
        return Err(error(format!(
            "record jsonmodel_type '{rec_type}' does not match wrapper record_type '{}'",
            input.record_type
        )));
    }

    let record_inheritance_has_type = input
        .record_inheritance_has_type
        .ok_or_else(|| error("record_inheritance_has_type must be explicitly present and boolean when prepare_pui_availability is true"))?;

    let conditional_additions =
        compute_pui_conditional_additions(&input.record_type, record_inheritance_has_type);

    let top_container_override = if input.record_type == "top_container" {
        let mut tc_map = Map::new();
        let linked_publish = record
            .get("is_linked_to_published_record")
            .cloned()
            .unwrap_or(Value::Null);
        tc_map.insert("publish".to_string(), linked_publish);
        tc_map.insert(
            "additions".to_string(),
            Value::Array(vec![
                Value::String("pui_container".to_string()),
                Value::String("pui".to_string()),
            ]),
        );
        Value::Object(tc_map)
    } else {
        Value::Null
    };

    let mut plan = Map::new();
    plan.insert(
        "marker".to_string(),
        Value::String(PUI_AVAILABILITY_METADATA_MARKER.to_string()),
    );
    plan.insert("uri".to_string(), Value::String(input.uri.clone()));
    plan.insert(
        "primary_type".to_string(),
        Value::String(input.record_type.clone()),
    );
    plan.insert(
        "record_inheritance_has_type".to_string(),
        Value::Bool(record_inheritance_has_type),
    );
    plan.insert(
        "conditional_additions".to_string(),
        Value::Array(
            conditional_additions
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    plan.insert("top_container_override".to_string(), top_container_override);

    document.insert(
        PUI_AVAILABILITY_DOCUMENTS_KEY.to_string(),
        Value::Object(plan),
    );

    match document.get_mut("_archivalist_rust_fields") {
        Some(Value::Array(fields)) => {
            fields.push(Value::String(PUI_AVAILABILITY_METADATA_MARKER.to_string()));
        }
        _ => {
            document.insert(
                "_archivalist_rust_fields".to_string(),
                Value::Array(vec![Value::String(
                    PUI_AVAILABILITY_METADATA_MARKER.to_string(),
                )]),
            );
        }
    }

    Ok(())
}

fn build_pui_common_projection(
    input: &IndexDocumentInput,
    record: &Map<String, Value>,
) -> Result<Value, IndexDocumentError> {
    // 1. Subjects helper patch
    let mut subjects_patch = Map::new();
    if ruby_truthy(record.get("subjects")) {
        let subjects_arr = record
            .get("subjects")
            .and_then(Value::as_array)
            .ok_or_else(|| error("subjects must be an array"))?;
        let mut titles = Vec::new();
        let mut uris = Vec::new();
        for subject in subjects_arr {
            let subject_obj = subject
                .as_object()
                .ok_or_else(|| error("subjects entries must be objects"))?;
            let resolved = subject_obj
                .get("_resolved")
                .and_then(Value::as_object)
                .ok_or_else(|| error("subjects entries must contain _resolved object"))?;
            if let Some(title) = resolved.get("title") {
                if !title.is_null() {
                    titles.push(title.clone());
                }
            }
            uris.push(subject_obj.get("ref").cloned().unwrap_or(Value::Null));
        }
        subjects_patch.insert("subjects".to_string(), Value::Array(titles));
        subjects_patch.insert("subject_uris".to_string(), Value::Array(uris));
    }

    // 2. Agents helper patch
    let mut agents_patch = Map::new();
    if ruby_truthy(record.get("linked_agents")) {
        let agents_arr = record
            .get("linked_agents")
            .and_then(Value::as_array)
            .ok_or_else(|| error("linked_agents must be an array"))?;
        let mut titles = Vec::new();
        let mut uris = Vec::new();
        let mut published_titles = Vec::new();
        let mut published_uris = Vec::new();
        let mut creators = Vec::new();
        for link in agents_arr {
            let link_obj = link
                .as_object()
                .ok_or_else(|| error("linked_agents entries must be objects"))?;
            let resolved = link_obj
                .get("_resolved")
                .and_then(Value::as_object)
                .ok_or_else(|| error("linked_agents entries must contain _resolved object"))?;
            let display_name = resolved
                .get("display_name")
                .and_then(Value::as_object)
                .ok_or_else(|| error("linked_agents._resolved must contain display_name object"))?;
            let sort_name = display_name
                .get("sort_name")
                .cloned()
                .unwrap_or(Value::Null);
            let uri = link_obj.get("ref").cloned().unwrap_or(Value::Null);

            titles.push(sort_name.clone());
            uris.push(uri.clone());

            if ruby_truthy(resolved.get("publish")) {
                published_titles.push(sort_name.clone());
                published_uris.push(uri);
            }
            if link_obj.get("role").and_then(Value::as_str) == Some("creator") {
                creators.push(sort_name);
            }
        }
        agents_patch.insert("agents".to_string(), Value::Array(titles));
        agents_patch.insert("agent_uris".to_string(), Value::Array(uris));
        agents_patch.insert(
            "published_agents".to_string(),
            Value::Array(published_titles),
        );
        agents_patch.insert(
            "published_agent_uris".to_string(),
            Value::Array(published_uris),
        );
        if !creators.is_empty() {
            agents_patch.insert("creators".to_string(), Value::Array(creators));
        }
    }

    // 3. Audit info helper patch
    let mut audit_patch = Map::new();
    for field in [
        "created_by",
        "last_modified_by",
        "user_mtime",
        "system_mtime",
        "create_time",
    ] {
        if let Some(val) = record.get(field) {
            audit_patch.insert(field.to_string(), val.clone());
        }
    }

    // 4. Years helper patch
    let mut years_patch = Map::new();
    if ruby_truthy(record.get("dates")) {
        let dates_arr = record
            .get("dates")
            .and_then(Value::as_array)
            .ok_or_else(|| error("dates must be an array"))?;
        let mut years = Vec::new();
        for date in dates_arr {
            let date_obj = date
                .as_object()
                .ok_or_else(|| error("dates entries must be objects"))?;
            let begin_val = date_obj.get("begin");
            let end_val = date_obj.get("end");
            let generated = generate_years_for_date_range(begin_val, end_val);
            for y in generated {
                if let Value::String(s) = y {
                    years.push(s);
                }
            }
        }
        if !years.is_empty() {
            years.sort();
            years.dedup();
            let first = format!("{:0>4}", years.first().unwrap());
            let last = format!("{:0>4}", years.last().unwrap());
            years_patch.insert(
                "years".to_string(),
                Value::Array(years.into_iter().map(Value::String).collect()),
            );
            years_patch.insert(
                "year_sort".to_string(),
                Value::String(format!("{first}{last}")),
            );
        } else {
            years_patch.insert("years".to_string(), Value::Array(Vec::new()));
        }

        let mut display_dates: Vec<&Value> = dates_arr
            .iter()
            .filter(|date| date.get("date_type").and_then(Value::as_str) == Some("inclusive"))
            .collect();
        if display_dates.is_empty() {
            display_dates.extend(dates_arr.iter());
        }
        let mut output_dates = Vec::new();
        for date in display_dates {
            let date_obj = date
                .as_object()
                .ok_or_else(|| error("dates entries must be objects"))?;
            if ruby_truthy(date_obj.get("expression")) {
                output_dates.push(date_obj.get("expression").cloned().unwrap_or(Value::Null));
            } else if date_obj.get("date_type").and_then(Value::as_str) == Some("single") {
                output_dates.push(date_obj.get("begin").cloned().unwrap_or(Value::Null));
            } else if ruby_truthy(date_obj.get("date_type")) {
                let begin_str = ruby_string(date_obj.get("begin"));
                let end_str = ruby_string(date_obj.get("end"));
                output_dates.push(Value::String(format!("{begin_str} - {end_str}")));
            }
        }
        years_patch.insert("dates".to_string(), Value::Array(output_dates));
    }

    // 5. Level helper patch
    let mut level_patch = Map::new();
    if let Some(level_val) = record.get("level") {
        let effective = if level_val.as_str() == Some("otherlevel") {
            record.get("other_level").cloned().unwrap_or(Value::Null)
        } else {
            level_val.clone()
        };
        level_patch.insert("level".to_string(), effective);
    }

    // 6. Summary helper patch
    let mut summary_patch = Map::new();
    if let Some(notes) = record.get("notes").and_then(Value::as_array) {
        if let Some(abstract_note) = notes
            .iter()
            .find(|note| note.get("type").and_then(Value::as_str) == Some("abstract"))
        {
            if let Some(content) = abstract_note.get("content").and_then(Value::as_array) {
                let mut strings = Vec::new();
                for val in content {
                    match val {
                        Value::Null => strings.push(String::new()),
                        Value::String(s) => strings.push(s.clone()),
                        Value::Bool(b) => strings.push(b.to_string()),
                        Value::Number(n) => strings.push(n.to_string()),
                        _ => return Err(error("unsupported non-scalar content in abstract note")),
                    }
                }
                summary_patch.insert("summary".to_string(), Value::String(strings.join("\n")));
            }
        } else if let Some(scope_note) = notes
            .iter()
            .find(|note| note.get("type").and_then(Value::as_str) == Some("scopecontent"))
        {
            if let Some(subnotes) = scope_note.get("subnotes").and_then(Value::as_array) {
                let mut strings = Vec::new();
                for sn in subnotes {
                    let sn_obj = sn
                        .as_object()
                        .ok_or_else(|| error("subnotes entries must be objects"))?;
                    match sn_obj.get("content") {
                        None | Some(Value::Null) => strings.push(String::new()),
                        Some(Value::String(s)) => strings.push(s.clone()),
                        Some(Value::Bool(b)) => strings.push(b.to_string()),
                        Some(Value::Number(n)) => strings.push(n.to_string()),
                        _ => {
                            return Err(error(
                                "unsupported non-scalar content in scopecontent subnote",
                            ))
                        }
                    }
                }
                summary_patch.insert("summary".to_string(), Value::String(strings.join("\n")));
            }
        }
    }

    // 7. Extents helper patch
    let mut extents_patch = Map::new();
    if ruby_truthy(record.get("extents")) {
        let extents_arr = record
            .get("extents")
            .and_then(Value::as_array)
            .ok_or_else(|| error("extents must be an array"))?;
        let whole: Vec<&Value> = extents_arr
            .iter()
            .filter(|extent| extent.get("portion").and_then(Value::as_str) == Some("whole"))
            .collect();
        let display = if whole.is_empty() {
            extents_arr.iter().collect::<Vec<_>>()
        } else {
            whole
        };
        let mut formatted = Vec::new();
        for extent in display {
            let extent_obj = extent
                .as_object()
                .ok_or_else(|| error("extents entries must be objects"))?;
            let number_str = ruby_string(extent_obj.get("number"));
            let type_str = ruby_string(extent_obj.get("extent_type"));
            formatted.push(Value::String(format!("{number_str} --- {type_str}")));
        }
        extents_patch.insert("extents".to_string(), Value::Array(formatted));
    }

    let mut pui_common = Map::new();
    pui_common.insert(
        "marker".to_string(),
        Value::String(PUI_COMMON_METADATA_MARKER.to_string()),
    );
    pui_common.insert("uri".to_string(), Value::String(input.uri.clone()));
    pui_common.insert(
        "primary_type".to_string(),
        Value::String(input.record_type.clone()),
    );
    pui_common.insert("subjects".to_string(), Value::Object(subjects_patch));
    pui_common.insert("agents".to_string(), Value::Object(agents_patch));
    pui_common.insert("audit_info".to_string(), Value::Object(audit_patch));
    pui_common.insert("years".to_string(), Value::Object(years_patch));
    pui_common.insert("level".to_string(), Value::Object(level_patch));
    pui_common.insert("summary".to_string(), Value::Object(summary_patch));
    pui_common.insert("extents".to_string(), Value::Object(extents_patch));

    Ok(Value::Object(pui_common))
}

fn add_pui_common_fields(
    document: &mut Map<String, Value>,
    input: &IndexDocumentInput,
    record: &Map<String, Value>,
) -> Result<(), IndexDocumentError> {
    let pui_common = build_pui_common_projection(input, record)?;
    document.insert(PUI_COMMON_DOCUMENTS_KEY.to_string(), pui_common);
    match document.get_mut("_archivalist_rust_fields") {
        Some(Value::Array(fields)) => {
            fields.push(Value::String(PUI_COMMON_METADATA_MARKER.to_string()));
        }
        _ => {
            document.insert(
                "_archivalist_rust_fields".to_string(),
                Value::Array(vec![Value::String(PUI_COMMON_METADATA_MARKER.to_string())]),
            );
        }
    }
    Ok(())
}

fn json_type_name(val: &Value) -> &'static str {
    match val {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "Integer"
            } else {
                "Float"
            }
        }
        Value::String(_) => "String",
        Value::Array(_) => "Array",
        Value::Object(_) => "Object",
    }
}

fn assert_json_type_strict_equal(
    expected: &Value,
    actual: &Value,
    path: &str,
) -> Result<(), IndexDocumentError> {
    match (expected, actual) {
        (Value::Null, Value::Null) => Ok(()),
        (Value::Bool(b1), Value::Bool(b2)) => {
            if b1 == b2 {
                Ok(())
            } else {
                Err(error(format!(
                    "{path}: boolean mismatch: expected {b1}, got {b2}"
                )))
            }
        }
        (Value::Number(n1), Value::Number(n2)) => {
            let n1_is_int = n1.is_i64() || n1.is_u64();
            let n2_is_int = n2.is_i64() || n2.is_u64();
            if n1_is_int != n2_is_int {
                return Err(error(format!(
                    "{path}: type mismatch between integer and float ({n1} vs {n2})"
                )));
            }
            if n1_is_int {
                let n1_i128 = n1
                    .as_i64()
                    .map(i128::from)
                    .or_else(|| n1.as_u64().map(i128::from));
                let n2_i128 = n2
                    .as_i64()
                    .map(i128::from)
                    .or_else(|| n2.as_u64().map(i128::from));
                if n1_i128 == n2_i128 {
                    Ok(())
                } else {
                    Err(error(format!(
                        "{path}: integer mismatch: expected {n1}, got {n2}"
                    )))
                }
            } else {
                let f1 = n1
                    .as_f64()
                    .ok_or_else(|| error(format!("{path}: unparseable float {n1}")))?;
                let f2 = n2
                    .as_f64()
                    .ok_or_else(|| error(format!("{path}: unparseable float {n2}")))?;
                if (f1 - f2).abs() < f64::EPSILON && f1.to_bits() == f2.to_bits() {
                    Ok(())
                } else {
                    Err(error(format!(
                        "{path}: float mismatch: expected {n1}, got {n2}"
                    )))
                }
            }
        }
        (Value::String(s1), Value::String(s2)) => {
            if s1 == s2 {
                Ok(())
            } else {
                Err(error(format!(
                    "{path}: string mismatch: expected {s1:?}, got {s2:?}"
                )))
            }
        }
        (Value::Array(a1), Value::Array(a2)) => {
            if a1.len() != a2.len() {
                return Err(error(format!(
                    "{path}: array length mismatch: expected {}, got {}",
                    a1.len(),
                    a2.len()
                )));
            }
            for (idx, (item1, item2)) in a1.iter().zip(a2.iter()).enumerate() {
                assert_json_type_strict_equal(item1, item2, &format!("{path}[{idx}]"))?;
            }
            Ok(())
        }
        (Value::Object(o1), Value::Object(o2)) => {
            if o1.len() != o2.len() {
                return Err(error(format!(
                    "{path}: object key count mismatch: expected {}, got {}",
                    o1.len(),
                    o2.len()
                )));
            }
            for (k, v1) in o1 {
                let v2 = o2.get(k).ok_or_else(|| {
                    error(format!(
                        "{path}: missing expected key {k:?} in auxiliary JSON"
                    ))
                })?;
                assert_json_type_strict_equal(v1, v2, &format!("{path}.{k}"))?;
            }
            for k in o2.keys() {
                if !o1.contains_key(k) {
                    return Err(error(format!(
                        "{path}: unexpected extra key {k:?} in auxiliary JSON"
                    )));
                }
            }
            Ok(())
        }
        (expected_val, actual_val) => Err(error(format!(
            "{path}: type mismatch: expected {}, got {}",
            json_type_name(expected_val),
            json_type_name(actual_val)
        ))),
    }
}

fn build_pui_auxiliary_document(input: &IndexDocumentInput) -> Result<Value, IndexDocumentError> {
    if !PUI_AUXILIARY_TYPES.contains(&input.record_type.as_str()) {
        return Err(error(format!(
            "invalid auxiliary record_type '{}'; expected one of {:?}",
            input.record_type, PUI_AUXILIARY_TYPES
        )));
    }

    let parent_id = input
        .pui_parent_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            error("pui_parent_id must be a non-empty string when prepare_pui_auxiliary is true")
        })?;

    if input.record.is_null() {
        return Err(error(
            "record payload must not be null for auxiliary document",
        ));
    }

    let aux_json_str = input
        .pui_auxiliary_json
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            error(
                "pui_auxiliary_json must be a non-empty string when prepare_pui_auxiliary is true",
            )
        })?;

    let parsed_aux: Value = serde_json::from_str(aux_json_str).map_err(|err| {
        error(format!(
            "pui_auxiliary_json must be a valid JSON string: {err}"
        ))
    })?;

    assert_json_type_strict_equal(&input.record, &parsed_aux, "pui_auxiliary_json")?;

    match input.record_type.as_str() {
        "resource_ordered_records" => {
            if !input.record.is_object() && !input.record.is_array() {
                return Err(error(
                    "record for resource_ordered_records must be an object or array",
                ));
            }
            if !input.uri.ends_with("/ordered_records") {
                return Err(error(format!(
                    "uri for resource_ordered_records must end with '/ordered_records': '{}'",
                    input.uri
                )));
            }
            if !input.uri.starts_with(parent_id) {
                return Err(error(format!(
                    "uri '{}' must start with parent_id '{}' for resource_ordered_records",
                    input.uri, parent_id
                )));
            }
        }
        "tree_root" => {
            if !input.record.is_object() {
                return Err(error("record for tree_root must be an object"));
            }
            if !input.uri.ends_with("/tree/root") {
                return Err(error(format!(
                    "uri for tree_root must end with '/tree/root': '{}'",
                    input.uri
                )));
            }
            if !input.uri.starts_with(parent_id) {
                return Err(error(format!(
                    "uri '{}' must start with parent_id '{}' for tree_root",
                    input.uri, parent_id
                )));
            }
        }
        "tree_waypoint" => {
            if !input.record.is_array() {
                return Err(error("record for tree_waypoint must be an array"));
            }
            if !input.uri.contains("/tree/waypoint_") {
                return Err(error(format!(
                    "uri for tree_waypoint must contain '/tree/waypoint_': '{}'",
                    input.uri
                )));
            }
        }
        "tree_node" => {
            if !input.record.is_object() {
                return Err(error("record for tree_node must be an object"));
            }
            if !input.uri.contains("/tree/node_") {
                return Err(error(format!(
                    "uri for tree_node must contain '/tree/node_': '{}'",
                    input.uri
                )));
            }
        }
        "tree_node_from_root" => {
            if !input.record.is_object() {
                return Err(error("record for tree_node_from_root must be an object"));
            }
            if !input.uri.contains("/tree/node_from_root_") {
                return Err(error(format!(
                    "uri for tree_node_from_root must contain '/tree/node_from_root_': '{}'",
                    input.uri
                )));
            }
        }
        _ => unreachable!(),
    }

    let mut document = Map::new();
    document.insert("id".to_string(), Value::String(input.uri.clone()));
    document.insert("uri".to_string(), Value::String(input.uri.clone()));
    document.insert(
        "pui_parent_id".to_string(),
        Value::String(parent_id.to_string()),
    );
    document.insert("publish".to_string(), Value::String("true".to_string()));
    document.insert(
        "primary_type".to_string(),
        Value::String(input.record_type.clone()),
    );
    document.insert(
        "types".to_string(),
        Value::Array(vec![Value::String("pui".to_string())]),
    );
    document.insert("json".to_string(), Value::String(aux_json_str.clone()));

    let mut marker = Map::new();
    marker.insert(
        "marker".to_string(),
        Value::String(PUI_AUXILIARY_METADATA_MARKER.to_string()),
    );
    marker.insert(
        "auxiliary_type".to_string(),
        Value::String(input.record_type.clone()),
    );
    document.insert(
        PUI_AUXILIARY_DOCUMENTS_KEY.to_string(),
        Value::Object(marker),
    );

    Ok(Value::Object(document))
}

pub fn build_index_document(input: IndexDocumentInput) -> Result<Value, IndexDocumentError> {
    if input.uri.trim().is_empty() {
        return Err(error("uri must not be empty"));
    }
    if input.record_type.trim().is_empty() {
        return Err(error("record_type must not be empty"));
    }
    if input.enrich && input.prepare_pui_final {
        return Err(error(
            "cannot combine enrich (staff) and prepare_pui_final (pui) in same record",
        ));
    }
    if input.enrich && input.prepare_pui_common_metadata {
        return Err(error(
            "cannot combine enrich (staff) and prepare_pui_common_metadata (pui) in same record",
        ));
    }
    if input.prepare_pui_final && input.prepare_pui_availability {
        return Err(error(
            "cannot combine prepare_pui_final (pui) and prepare_pui_availability in same record",
        ));
    }
    if input.prepare_pui_availability && input.prepare_pui_common_metadata {
        return Err(error(
            "cannot combine prepare_pui_availability and prepare_pui_common_metadata in same record",
        ));
    }
    if input.prepare_pui_auxiliary {
        if input.enrich
            || input.prepare_pui_final
            || input.prepare_pui_availability
            || input.prepare_pui_common_metadata
        {
            return Err(error(
                "cannot combine prepare_pui_auxiliary with other preparation modes",
            ));
        }
        if input.arks_enabled
            || !input.dynamic_enum_fields.is_empty()
            || input.accession_date_unknown_label.is_some()
            || input.parsed_accession_date_year.is_some()
            || input.pui_candidate_record.is_some()
            || input.record_inheritance_has_type.is_some()
            || input.record_inheritance_plan.is_some()
        {
            return Err(error(
                "cannot set staff enrichment or pui final/availability fields when prepare_pui_auxiliary is true",
            ));
        }
        return build_pui_auxiliary_document(&input);
    }
    if !input.prepare_pui_final && input.record_inheritance_plan.is_some() {
        return Err(error(
            "record_inheritance_plan is only allowed when prepare_pui_final is true",
        ));
    }
    if input.pui_parent_id.is_some() {
        return Err(error(
            "pui_parent_id is only allowed when prepare_pui_auxiliary is true",
        ));
    }
    if input.pui_auxiliary_json.is_some() {
        return Err(error(
            "pui_auxiliary_json is only allowed when prepare_pui_auxiliary is true",
        ));
    }

    let record = input
        .record
        .as_object()
        .ok_or_else(|| error("record must be a JSON object"))?;

    if input.prepare_pui_availability {
        let uri_val = record
            .get("uri")
            .ok_or_else(|| error("record.uri is required when prepare_pui_availability is true"))?;
        let rec_uri = uri_val
            .as_str()
            .ok_or_else(|| error("record.uri must be a non-empty string"))?;
        if rec_uri.trim().is_empty() {
            return Err(error("record.uri must not be empty"));
        }
        if rec_uri != input.uri {
            return Err(error(format!(
                "record uri '{rec_uri}' does not match wrapper uri '{}'",
                input.uri
            )));
        }

        let type_val = record.get("jsonmodel_type").ok_or_else(|| {
            error("record.jsonmodel_type is required when prepare_pui_availability is true")
        })?;
        let rec_type = type_val
            .as_str()
            .ok_or_else(|| error("record.jsonmodel_type must be a non-empty string"))?;
        if rec_type.trim().is_empty() {
            return Err(error("record.jsonmodel_type must not be empty"));
        }
        if rec_type != input.record_type {
            return Err(error(format!(
                "record jsonmodel_type '{rec_type}' does not match wrapper record_type '{}'",
                input.record_type
            )));
        }

        if input.record_inheritance_has_type.is_none() {
            return Err(error("record_inheritance_has_type must be explicitly present and boolean when prepare_pui_availability is true"));
        }
    } else if input.prepare_pui_common_metadata {
        let uri_val = record.get("uri").ok_or_else(|| {
            error("record.uri is required when prepare_pui_common_metadata is true")
        })?;
        let rec_uri = uri_val
            .as_str()
            .ok_or_else(|| error("record.uri must be a non-empty string"))?;
        if rec_uri.trim().is_empty() {
            return Err(error("record.uri must not be empty"));
        }
        if rec_uri != input.uri {
            return Err(error(format!(
                "record uri '{rec_uri}' does not match wrapper uri '{}'",
                input.uri
            )));
        }

        let type_val = record.get("jsonmodel_type").ok_or_else(|| {
            error("record.jsonmodel_type is required when prepare_pui_common_metadata is true")
        })?;
        let rec_type = type_val
            .as_str()
            .ok_or_else(|| error("record.jsonmodel_type must be a non-empty string"))?;
        if rec_type.trim().is_empty() {
            return Err(error("record.jsonmodel_type must not be empty"));
        }
        if rec_type != input.record_type {
            return Err(error(format!(
                "record jsonmodel_type '{rec_type}' does not match wrapper record_type '{}'",
                input.record_type
            )));
        }
    } else if let Some(record_uri) = record.get("uri").and_then(Value::as_str) {
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

    let collection_management = if input.enrich {
        collection_management_document(&input.uri, &input.record_type, &input.repository, record)?
    } else {
        None
    };

    let mut document = Map::new();
    document.insert("id".to_string(), Value::String(input.uri.clone()));
    document.insert("uri".to_string(), Value::String(input.uri.clone()));
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
    document.insert("repository".to_string(), input.repository.clone());

    if input.enrich {
        add_enriched_fields(&mut document, &input, record)?;
    } else if input.prepare_pui_final {
        add_pui_final_fields(&mut document, &input)?;
    }

    if input.prepare_pui_common_metadata {
        add_pui_common_fields(&mut document, &input, record)?;
    }

    if input.prepare_pui_availability {
        add_pui_availability_fields(&mut document, &input, record)?;
    }

    if let Some(child) = collection_management {
        document.insert(
            COLLECTION_MANAGEMENT_DOCUMENTS_KEY.to_string(),
            Value::Array(vec![child]),
        );
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
            ..Default::default()
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
                "resource_archival_object_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
        };
        enriched.enrich = true;

        let document = build_index_document(enriched).unwrap();
        assert_eq!(document["title"], "Series I: Correspondence");
        assert_eq!(document["title_sort"], "Original title");
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
    fn test_title_sort_semantics_and_precedence() {
        // 1. Archival object: uses record title, NOT display_string
        let doc = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/1".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/1",
                "title": "Series 1: ledgers",
                "display_string": "Series 1: ledgers, 1899-1926",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc["title"], "Series 1: ledgers, 1899-1926");
        assert_eq!(doc["title_sort"], "Series 1 ledgers");

        // 2. finding_aid_filing_title override when non-nil and non-empty
        let doc_filing = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/resources/1".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/1",
                "title": "Personal Papers",
                "finding_aid_filing_title": "Smith, John: Personal Papers",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_filing["title_sort"], "Smith John Personal Papers");

        // 3. finding_aid_filing_title empty fallback to title
        let doc_empty_filing = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/resources/2".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/2",
                "title": "Personal Papers",
                "finding_aid_filing_title": "",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_empty_filing["title_sort"], "Personal Papers");

        // 4. finding_aid_filing_title null fallback to title
        let doc_null_filing = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/resources/3".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/3",
                "title": "Personal Papers",
                "finding_aid_filing_title": Value::Null,
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_null_filing["title_sort"], "Personal Papers");

        // 5. Null title when no filing title -> null title_sort
        let doc_null_title = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/2".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/2",
                "title": Value::Null,
                "display_string": "Undated item",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_null_title["title_sort"], Value::Null);

        // 6. Absent title when no filing title -> null title_sort
        let doc_absent_title = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/3".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/3",
                "display_string": "Undated item",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_absent_title["title_sort"], Value::Null);

        // 7. Empty title -> empty title_sort
        let doc_empty_title = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/4".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/4",
                "title": "",
                "display_string": "",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_empty_title["title_sort"], "");

        // 8. HTML tag stripping, hyphen replacement, Unicode and whitespace
        let doc_complex = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/5".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/5",
                "title": "  <b>Section 1</b>: post-war & <i>café naïve</i>_notes...  ",
                "display_string": "Section 1",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            doc_complex["title_sort"],
            "Section 1 post war café naïve_notes"
        );

        // 9. Precedence preservation: existing truthy title_sort is not overwritten
        let mut doc_map = Map::new();
        doc_map.insert(
            "title_sort".to_string(),
            Value::String("Explicitly Preserved Sort".to_string()),
        );
        let mut rec_map = Map::new();
        rec_map.insert(
            "title".to_string(),
            Value::String("Ignored Title".to_string()),
        );
        apply_title_sort(&mut doc_map, &rec_map);
        assert_eq!(doc_map["title_sort"], "Explicitly Preserved Sort");
    }

    #[test]
    fn test_add_dates_and_years_semantics() {
        // 1. dates: [] (empty array) -> years: [], dates: [], no year_sort
        let doc_empty_dates = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/10".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/10",
                "title": "Item with empty dates",
                "dates": [],
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_empty_dates["years"], json!([]));
        assert_eq!(doc_empty_dates["dates"], json!([]));
        assert!(doc_empty_dates.get("year_sort").is_none());

        // 2. dates absent -> no years, no dates, no year_sort
        let doc_no_dates = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/11".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/11",
                "title": "Item with absent dates",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert!(doc_no_dates.get("years").is_none());
        assert!(doc_no_dates.get("dates").is_none());
        assert!(doc_no_dates.get("year_sort").is_none());

        // 3. dates: null -> no years, no dates, no year_sort
        let doc_null_dates = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/12".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/12",
                "title": "Item with null dates",
                "dates": Value::Null,
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert!(doc_null_dates.get("years").is_none());
        assert!(doc_null_dates.get("dates").is_none());
        assert!(doc_null_dates.get("year_sort").is_none());

        // 4. dates: false -> no years, no dates, no year_sort
        let doc_false_dates = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/13".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/13",
                "title": "Item with false dates",
                "dates": false,
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert!(doc_false_dates.get("years").is_none());
        assert!(doc_false_dates.get("dates").is_none());
        assert!(doc_false_dates.get("year_sort").is_none());

        // 5. Strict type checks: non-null, non-false, non-array dates rejects with error
        let err_bool = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/14".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/14",
                "dates": true,
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err_bool.to_string(), "field 'dates' must be an array");

        let err_str = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/15".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/15",
                "dates": "1999",
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err_str.to_string(), "field 'dates' must be an array");

        let err_entry = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/16".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/16",
                "dates": [123],
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap_err();
        assert_eq!(err_entry.to_string(), "dates entries must be objects");

        // 6. dates with unparseable year -> years: [], expression in dates, no year_sort
        let doc_unparseable = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/17".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/17",
                "title": "Undated",
                "dates": [{
                    "date_type": "inclusive",
                    "expression": "circa unknown"
                }],
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_unparseable["years"], json!([]));
        assert_eq!(doc_unparseable["dates"], json!(["circa unknown"]));
        assert!(doc_unparseable.get("year_sort").is_none());

        // 7. dates with parseable range -> years populated, year_sort populated
        let doc_ranged = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/18".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/18",
                "title": "Ranged dates",
                "dates": [{
                    "date_type": "inclusive",
                    "begin": "1912",
                    "end": "1914"
                }],
                "publish": true,
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_ranged["years"], json!(["1912", "1913", "1914"]));
        assert_eq!(doc_ranged["year_sort"], "19121914");
        assert_eq!(doc_ranged["dates"], json!(["1912 - 1914"]));
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
                "lang_materials": [{"language_and_script": {"language": "gle"}}],
                "accession_date": "2026-10-17"
            }),
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
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
        assert_eq!(document["accession_date"], "2026-10-17");
        assert_eq!(document["accession_date_year"], 2026);
        assert_eq!(
            document["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "accession_digital_object_metadata",
                "accession_date_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "accession_digital_object_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
            json!([
                "common_metadata",
                "fullrecord",
                "agent_subject_metadata",
                "dynamic_enum_and_ark_metadata"
            ])
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
            ..Default::default()
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
            json!([
                "common_metadata",
                "fullrecord",
                "agent_subject_metadata",
                "dynamic_enum_and_ark_metadata"
            ])
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
            ..Default::default()
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
                "component_classification_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "component_classification_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "repository_event_location_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "repository_event_location_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "repository_event_location_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
            ..Default::default()
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
                "profile_assessment_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "profile_assessment_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
                "profile_assessment_metadata",
                "dynamic_enum_and_ark_metadata"
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
            ..Default::default()
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
            ..Default::default()
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
            json!([
                "common_metadata",
                "fullrecord",
                "top_container_metadata",
                "dynamic_enum_and_ark_metadata"
            ])
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
            ..Default::default()
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
            json!([
                "common_metadata",
                "fullrecord",
                "top_container_metadata",
                "dynamic_enum_and_ark_metadata"
            ])
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
            ..Default::default()
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
            ..Default::default()
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
    fn builds_job_staff_index_fields_for_full_and_minimal_records() {
        let report_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/14".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/14",
                "job_type": "report_job",
                "job": {
                    "jsonmodel_type": "report_job",
                    "report_type": "assessment_list_report",
                    "format": "pdf"
                },
                "status": "queued",
                "owner": "admin",
                "queue_position": 2,
                "time_submitted": "2026-10-17 12:00:00 UTC",
                "created_by": "admin",
                "external_ids": [{"external_id": "job-ext-14"}],
                "ancestors": [{"ref": "/repositories/2/resources/7"}],
                "lang_materials": [{"language_and_script": {"language": "eng"}}]
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();

        assert_eq!(report_job["id"], "/repositories/2/jobs/14");
        assert_eq!(report_job["uri"], "/repositories/2/jobs/14");
        assert_eq!(report_job["primary_type"], "job");
        assert_eq!(report_job["title"], Value::Null);
        assert_eq!(
            report_job["types"],
            json!(["job", "report_job", "assessment_list_report"])
        );
        assert_eq!(report_job["job_type"], "report_job");
        assert_eq!(report_job["report_type"], "assessment_list_report");
        assert_eq!(report_job["job_report_type"], "assessment_list_report");
        assert_eq!(report_job["status"], "queued");
        assert_eq!(report_job["owner"], "admin");
        assert_eq!(report_job["queue_position"], 2);
        assert_eq!(report_job["created_by"], "admin");
        assert_eq!(report_job["external_id"], json!(["job-ext-14"]));
        assert_eq!(
            report_job["ancestors"],
            json!(["/repositories/2/resources/7"])
        );
        assert_eq!(report_job["langcode"], json!(["eng"]));
        assert!(report_job.get("time_submitted").is_none());
        assert!(report_job.get("time_started").is_none());
        assert!(report_job.get("time_finished").is_none());
        assert!(report_job.get("files").is_none());
        assert!(report_job.get("job_data").is_none());
        assert_eq!(
            report_job["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "job_metadata",
                "dynamic_enum_and_ark_metadata"
            ])
        );

        let minimal_import_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/15".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/15",
                "job_type": "import_job",
                "job": {
                    "jsonmodel_type": "import_job",
                    "filenames": ["ead.xml"]
                },
                "status": "completed",
                "owner": "archivist"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();

        assert_eq!(
            minimal_import_job["types"],
            json!(["job", "import_job", Value::Null])
        );
        assert_eq!(minimal_import_job["job_type"], "import_job");
        assert_eq!(minimal_import_job["report_type"], Value::Null);
        assert_eq!(minimal_import_job["job_report_type"], "import_job");
        assert_eq!(minimal_import_job["status"], "completed");
        assert_eq!(minimal_import_job["owner"], "archivist");
        assert_eq!(minimal_import_job["queue_position"], Value::Null);
        assert_eq!(minimal_import_job["external_id"], json!([]));
        assert_eq!(minimal_import_job["ancestors"], json!([]));
        assert_eq!(minimal_import_job["langcode"], json!([]));
        assert_eq!(
            minimal_import_job["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "job_metadata",
                "dynamic_enum_and_ark_metadata"
            ])
        );

        let falsy_report_type_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/16".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/16",
                "job_type": "batch_delete_job",
                "job": {
                    "jsonmodel_type": "batch_delete_job",
                    "report_type": false
                },
                "status": "failed",
                "owner": "admin"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            falsy_report_type_job["types"],
            json!(["job", "batch_delete_job", false])
        );
        assert_eq!(falsy_report_type_job["report_type"], false);
        assert_eq!(falsy_report_type_job["job_report_type"], "batch_delete_job");

        let empty_report_type_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/17".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/17",
                "job_type": "report_job",
                "job": {
                    "jsonmodel_type": "report_job",
                    "report_type": ""
                },
                "status": "running",
                "owner": "admin"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(empty_report_type_job["report_type"], "");
        assert_eq!(empty_report_type_job["job_report_type"], "");

        let legacy_string_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/18".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/18",
                "job_type": "removed_plugin_job",
                "job": "removed_plugin_job",
                "inactive_record": true,
                "status": "completed",
                "owner": "admin"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            legacy_string_job["types"],
            json!(["job", "removed_plugin_job", Value::Null])
        );
        assert_eq!(legacy_string_job["job_type"], "removed_plugin_job");
        assert_eq!(legacy_string_job["report_type"], Value::Null);
        assert_eq!(legacy_string_job["job_report_type"], "removed_plugin_job");

        let string_with_report_type_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/19".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/19",
                "job_type": "legacy_job",
                "job": "legacy_report_type_job",
                "status": "completed",
                "owner": "admin"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            string_with_report_type_job["types"],
            json!(["job", "legacy_job", "report_type"])
        );
        assert_eq!(string_with_report_type_job["job_type"], "legacy_job");
        assert_eq!(string_with_report_type_job["report_type"], "report_type");
        assert_eq!(
            string_with_report_type_job["job_report_type"],
            "report_type"
        );

        let explicit_null_report_type_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/20".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/20",
                "job_type": "export_job",
                "job": {
                    "jsonmodel_type": "export_job",
                    "report_type": Value::Null
                },
                "status": "completed",
                "owner": "admin"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            explicit_null_report_type_job["types"],
            json!(["job", "export_job", Value::Null])
        );
        assert_eq!(explicit_null_report_type_job["report_type"], Value::Null);
        assert_eq!(
            explicit_null_report_type_job["job_report_type"],
            "export_job"
        );

        let missing_job_type_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/21".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/21",
                "job": {
                    "jsonmodel_type": "unknown_job"
                },
                "status": "queued",
                "owner": "admin"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            missing_job_type_job["types"],
            json!(["job", Value::Null, Value::Null])
        );
        assert_eq!(missing_job_type_job["job_type"], Value::Null);
        assert_eq!(missing_job_type_job["report_type"], Value::Null);
        assert_eq!(missing_job_type_job["job_report_type"], Value::Null);
    }

    #[test]
    fn keeps_unmigrated_types_and_unenriched_requests_free_of_type_specific_markers() {
        let custom_record = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/custom_records/5".to_string(),
            record_type: "custom_record".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/custom_records/5",
                "title": "Custom 5"
            }),
            enrich: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            custom_record["_archivalist_rust_fields"],
            json!([
                "common_metadata",
                "fullrecord",
                "dynamic_enum_and_ark_metadata"
            ])
        );
        assert!(
            !custom_record["_archivalist_rust_fields"]
                .as_array()
                .unwrap()
                .contains(&json!("job_metadata")),
            "custom record should not contain job_metadata marker"
        );

        let unenriched_job = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/5".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/5",
                "title": "Envelope job title",
                "job_type": "import_job",
                "job": {"jsonmodel_type": "import_job"},
                "status": "queued",
                "owner": "admin"
            }),
            enrich: false,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(unenriched_job["title"], "Envelope job title");
        assert_eq!(unenriched_job["types"], json!(["job"]));
        assert!(unenriched_job.get("_archivalist_rust_fields").is_none());
        assert!(unenriched_job.get("job_type").is_none());
        assert!(unenriched_job.get("job_report_type").is_none());

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
            ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        })
        .is_err());
        assert!(build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/5".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/5"
            }),
            enrich: true,
            ..Default::default()
        })
        .is_err());
        assert!(build_index_document(IndexDocumentInput {
            uri: "/repositories/2/jobs/5".to_string(),
            record_type: "job".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/jobs/5",
                "job": ["invalid-array-job"]
            }),
            enrich: true,
            ..Default::default()
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
    fn builds_collection_management_child_document_for_full_and_minimal_records() {
        // Full resource parent with collection_management
        let full_cm = json!({
            "uri": "/repositories/2/collection_management/10",
            "processing_priority": " high ",
            "processing_status": " completed ",
            "processing_hours_total": " 40 ",
            "processing_funding_source": " grant-funded ",
            "processors": " Alice and Bob ",
            "created_by": " admin ",
            "last_modified_by": " archivist ",
            "system_mtime": "2026-01-01T00:00:00Z",
            "user_mtime": "2026-01-01T00:00:00Z",
            "create_time": "2026-01-01T00:00:00Z"
        });
        let mut full_resource = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": " <emph render='italic'>Papers</emph> - Collection 1 ",
            "suppressed": true,
            "collection_management": full_cm
        }));
        full_resource.enrich = true;
        let full_doc = build_index_document(full_resource).unwrap();
        let cm_children = full_doc[COLLECTION_MANAGEMENT_DOCUMENTS_KEY]
            .as_array()
            .expect("child documents array");
        assert_eq!(cm_children.len(), 1);
        let child = &cm_children[0];
        assert_eq!(child["id"], "/repositories/2/collection_management/10");
        assert_eq!(child["uri"], "/repositories/2/collection_management/10");
        assert_eq!(child["parent_id"], "/repositories/2/resources/7");
        assert_eq!(
            child["parent_title"],
            "<emph render='italic'>Papers</emph> - Collection 1"
        );
        assert_eq!(child["parent_type"], "resource");
        assert_eq!(
            child["title"],
            "<emph render='italic'>Papers</emph> - Collection 1"
        );
        assert_eq!(child["title_sort"], "Papers Collection 1");
        assert_eq!(child["primary_type"], "collection_management");
        assert_eq!(child["types"], json!(["collection_management"]));
        assert_eq!(child["suppressed"], true);
        assert_eq!(child["repository"], 2);
        // Stripped processing strings:
        assert_eq!(child["processing_priority"], "high");
        assert_eq!(child["processing_status"], "completed");
        assert_eq!(child["processing_hours_total"], "40");
        assert_eq!(child["processing_funding_source"], "grant-funded");
        assert_eq!(child["processors"], "Alice and Bob");
        // Audit fields from fullrecord_excludes:
        assert_eq!(child["created_by"], " admin ");
        assert_eq!(child["last_modified_by"], " archivist ");
        assert_eq!(child["system_mtime"], "2026-01-01T00:00:00Z");
        assert_eq!(child["user_mtime"], "2026-01-01T00:00:00Z");
        assert_eq!(child["create_time"], "2026-01-01T00:00:00Z");
        // Verify JSON string parses and has stripped processing fields
        let parsed_json: Value = serde_json::from_str(child["json"].as_str().unwrap()).unwrap();
        assert_eq!(parsed_json["processing_priority"], "high");
        assert_eq!(
            parsed_json["uri"],
            "/repositories/2/collection_management/10"
        );

        // Minimal accession parent with minimal collection_management
        let minimal_cm = json!({
            "uri": "/repositories/2/collection_management/11"
        });
        let minimal_accession = IndexDocumentInput {
            uri: "/repositories/2/accessions/2".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/2",
                "display_string": "Accession 2026-01",
                "title": Value::Null,
                "collection_management": minimal_cm,
                "accession_date": "2026-01-01"
            }),
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
        };
        let acc_doc = build_index_document(minimal_accession).unwrap();
        let acc_children = acc_doc[COLLECTION_MANAGEMENT_DOCUMENTS_KEY]
            .as_array()
            .unwrap();
        assert_eq!(acc_children.len(), 1);
        let acc_child = &acc_children[0];
        assert_eq!(acc_child["id"], "/repositories/2/collection_management/11");
        assert_eq!(acc_child["uri"], "/repositories/2/collection_management/11");
        assert_eq!(acc_child["parent_id"], "/repositories/2/accessions/2");
        assert_eq!(acc_child["parent_title"], "Accession 2026-01");
        assert_eq!(acc_child["parent_type"], "accession");
        assert_eq!(acc_child["title"], "Accession 2026-01");
        assert_eq!(acc_child["title_sort"], "Accession 2026 01");
        assert_eq!(acc_child["primary_type"], "collection_management");
        assert_eq!(acc_child["types"], json!(["collection_management"]));
        assert_eq!(acc_child["processing_priority"], Value::Null);
        assert_eq!(acc_child["created_by"], Value::Null);

        // Digital object parent with collection_management
        let do_cm = json!({
            "uri": "/repositories/2/collection_management/12"
        });
        let digital_object = IndexDocumentInput {
            uri: "/repositories/2/digital_objects/3".to_string(),
            record_type: "digital_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/digital_objects/3",
                "title": "Digital Photo Collection",
                "collection_management": do_cm
            }),
            enrich: true,
            ..Default::default()
        };
        let do_doc = build_index_document(digital_object).unwrap();
        let do_child = &do_doc[COLLECTION_MANAGEMENT_DOCUMENTS_KEY]
            .as_array()
            .unwrap()[0];
        assert_eq!(do_child["parent_type"], "digital_object");
        assert_eq!(do_child["title"], "Digital Photo Collection");
    }

    #[test]
    fn collection_management_edge_cases_and_withholding() {
        // Missing CM subrecord: no handoff key
        let mut no_cm = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Resource without CM"
        }));
        no_cm.enrich = true;
        let no_cm_doc = build_index_document(no_cm).unwrap();
        assert!(!no_cm_doc
            .as_object()
            .unwrap()
            .contains_key(COLLECTION_MANAGEMENT_DOCUMENTS_KEY));

        // Enrichment disabled: no handoff key even when CM is present
        let mut unenriched = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Unenriched Resource",
            "collection_management": { "uri": "/repositories/2/collection_management/20" }
        }));
        unenriched.enrich = false;
        let unenriched_doc = build_index_document(unenriched).unwrap();
        assert!(!unenriched_doc
            .as_object()
            .unwrap()
            .contains_key(COLLECTION_MANAGEMENT_DOCUMENTS_KEY));

        // Unsupported parent type (e.g. archival_object): withheld, no handoff key
        let unsupported_type = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/4".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/4",
                "display_string": "Item 4",
                "collection_management": { "uri": "/repositories/2/collection_management/21" }
            }),
            enrich: true,
            ..Default::default()
        };
        let unsupported_doc = build_index_document(unsupported_type).unwrap();
        assert!(!unsupported_doc
            .as_object()
            .unwrap()
            .contains_key(COLLECTION_MANAGEMENT_DOCUMENTS_KEY));

        // Empty CM uri: withheld, no handoff key
        let mut empty_cm_uri = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Resource with empty CM uri",
            "collection_management": { "uri": "  " }
        }));
        empty_cm_uri.enrich = true;
        let empty_uri_doc = build_index_document(empty_cm_uri).unwrap();
        assert!(!empty_uri_doc
            .as_object()
            .unwrap()
            .contains_key(COLLECTION_MANAGEMENT_DOCUMENTS_KEY));

        // Title with non-standard Unicode characters (Greek / Cyrillic): withheld, no handoff key
        let mut non_standard_letters = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "Архив / Επιστολές",
            "collection_management": { "uri": "/repositories/2/collection_management/22" }
        }));
        non_standard_letters.enrich = true;
        let non_std_doc = build_index_document(non_standard_letters).unwrap();
        assert!(!non_std_doc
            .as_object()
            .unwrap()
            .contains_key(COLLECTION_MANAGEMENT_DOCUMENTS_KEY));

        // Title is null and display_string is null: child title and title_sort are null
        let mut null_title = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": Value::Null,
            "display_string": Value::Null,
            "collection_management": { "uri": "/repositories/2/collection_management/23" }
        }));
        null_title.enrich = true;
        let null_title_doc = build_index_document(null_title).unwrap();
        let null_child = &null_title_doc[COLLECTION_MANAGEMENT_DOCUMENTS_KEY]
            .as_array()
            .unwrap()[0];
        assert_eq!(null_child["parent_title"], Value::Null);
        assert_eq!(null_child["title"], Value::Null);
        assert_eq!(null_child["title_sort"], Value::Null);

        // Title is "" (empty string) and display_string is "Fallback": Ruby truthiness keeps ""
        let mut empty_title = input(json!({
            "uri": "/repositories/2/resources/7",
            "title": "",
            "display_string": "Fallback Display",
            "collection_management": { "uri": "/repositories/2/collection_management/24" }
        }));
        empty_title.enrich = true;
        let empty_title_doc = build_index_document(empty_title).unwrap();
        let empty_child = &empty_title_doc[COLLECTION_MANAGEMENT_DOCUMENTS_KEY]
            .as_array()
            .unwrap()[0];
        assert_eq!(empty_child["title"], "");
        assert_eq!(empty_child["title_sort"], "");
    }

    #[test]
    fn enforces_nonempty_bounded_batches() {
        assert!(build_index_document_batch(IndexDocumentBatchRequest { records: vec![] }).is_err());
        let records = (0..=MAX_INDEX_DOCUMENT_BATCH)
            .map(|_| input(json!({"uri": "/repositories/2/resources/7"})))
            .collect();
        assert!(build_index_document_batch(IndexDocumentBatchRequest { records }).is_err());
    }

    #[test]
    fn dynamic_enums_nested_hashes_and_arrays() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/10".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/10",
                "title": "Nested Enums Resource",
                "publish": true,
                "rights_statements": [
                    {
                        "rights_type": "copyright",
                        "acts": [
                            { "status": "active" },
                            { "status": "expired" }
                        ]
                    }
                ],
                "instances": [
                    {
                        "instance_type": "text",
                        "sub_container": {
                            "type_2": "folder"
                        }
                    }
                ]
            }),
            enrich: true,
            dynamic_enum_fields: vec![
                "rights_type".to_string(),
                "status".to_string(),
                "instance_type".to_string(),
                "type_2".to_string(),
                "nonexistent".to_string(),
            ],
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["rights_type_enum_s"], json!(["copyright"]));
        assert_eq!(doc["status_enum_s"], json!(["active", "expired"]));
        assert_eq!(doc["instance_type_enum_s"], json!(["text"]));
        assert_eq!(doc["type_2_enum_s"], json!(["folder"]));
        assert!(doc.get("nonexistent_enum_s").is_none());
        assert!(doc["_archivalist_rust_fields"]
            .as_array()
            .unwrap()
            .contains(&json!("dynamic_enum_and_ark_metadata")));
    }

    #[test]
    fn dynamic_enums_excludes_resolved_subtrees() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/11".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/11",
                "title": "Resolved Exclusion Resource",
                "publish": true,
                "rights_type": "in_copyright",
                "_resolved": {
                    "rights_type": "should_be_ignored_copyright",
                    "status": "should_be_ignored_status",
                    "items": { "type": "should_be_ignored_type" }
                },
                "subrecord": {
                    "_resolved": {
                        "status": "nested_ignored_status"
                    }
                }
            }),
            enrich: true,
            dynamic_enum_fields: vec![
                "rights_type".to_string(),
                "status".to_string(),
                "type".to_string(),
            ],
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["rights_type_enum_s"], json!(["in_copyright"]));
        assert!(doc.get("status_enum_s").is_none());
        assert!(doc.get("type_enum_s").is_none());
    }

    #[test]
    fn dynamic_enums_preserves_stable_order_and_deduplicates() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/12".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/12",
                "title": "Order and Dedupe",
                "publish": true,
                "notes": [
                    { "status": "draft" },
                    { "status": "reviewed" },
                    { "status": "draft" },
                    { "status": "approved" }
                ],
                "array_field": ["val1", "val2"],
                "sub": {
                    "array_field": "val1"
                }
            }),
            enrich: true,
            dynamic_enum_fields: vec!["status".to_string(), "array_field".to_string()],
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(
            doc["status_enum_s"],
            json!(["draft", "reviewed", "approved"])
        );
        // array ["val1", "val2"] added, then scalar "val1" added (not equal to array), then flattened
        assert_eq!(doc["array_field_enum_s"], json!(["val1", "val2", "val1"]));
    }

    #[test]
    fn dynamic_enums_handles_multiple_keys_in_same_and_different_objects() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/13".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/13",
                "title": "Multiple Keys Resource",
                "publish": true,
                "level": "collection",
                "resource_type": "records",
                "sub": {
                    "level": "series",
                    "resource_type": "papers"
                }
            }),
            enrich: true,
            dynamic_enum_fields: vec!["level".to_string(), "resource_type".to_string()],
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["level_enum_s"], json!(["collection", "series"]));
        assert_eq!(doc["resource_type_enum_s"], json!(["records", "papers"]));
    }

    #[test]
    fn dynamic_enums_projects_items_type_into_type_enum_s() {
        // When "type" is not in dynamic_enum_fields, only items containing a Hash with "type"
        // are projected into type_enum_s (native Pass 2 behavior).
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/14".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/14",
                "title": "Items Type Resource",
                "publish": true,
                "type": "outer_type",
                "notes": [
                    {
                        "items": { "type": "inner_hash_type" }
                    },
                    {
                        "items": [
                            { "type": "array_item_type_not_a_hash_under_items" }
                        ]
                    }
                ]
            }),
            enrich: true,
            dynamic_enum_fields: vec![],
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["type_enum_s"], json!(["inner_hash_type"]));
    }

    #[test]
    fn dynamic_enums_omits_absent_fields_and_preserves_empty_arrays() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/15".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/15",
                "title": "Absent and Empty",
                "publish": true,
                "empty_field": []
            }),
            enrich: true,
            dynamic_enum_fields: vec!["empty_field".to_string(), "missing_field".to_string()],
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["empty_field_enum_s"], json!([]));
        assert!(doc.get("missing_field_enum_s").is_none());
    }

    #[test]
    fn projects_arks_with_current_and_previous_ordering_and_trim_semantics() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/16".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/16",
                "title": "ARK Resource",
                "publish": true,
                "ark_name": {
                    "current": "https://n2t.net/ark:/12345/current_val",
                    "previous": [
                        "ark:/12345/prev_one",
                        "https://archive.org/ark:/12345/prev_two"
                    ]
                }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(
            doc["ark_name"],
            json!([
                "ark:/12345/current_val",
                "ark:/12345/prev_one",
                "ark:/12345/prev_two"
            ])
        );

        // Null current: only previous values
        let null_curr_input = IndexDocumentInput {
            uri: "/repositories/2/resources/17".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/17",
                "title": "Null Current ARK Resource",
                "publish": true,
                "ark_name": {
                    "current": null,
                    "previous": ["ark:/12345/prev_only"]
                }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        let null_curr_doc = build_index_document(null_curr_input).unwrap();
        assert_eq!(null_curr_doc["ark_name"], json!(["ark:/12345/prev_only"]));
    }

    #[test]
    fn projects_arks_with_multiple_ark_substrings() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/18".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/18",
                "title": "Multiple ARK Substrings Resource",
                "publish": true,
                "ark_name": {
                    "current": "http://example.org/ark:/proxy/ark:/final_val",
                    "previous": []
                }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["ark_name"], json!(["ark:/final_val"]));
    }

    #[test]
    fn projects_dynamic_enums_and_arks_with_unicode() {
        let input_doc = IndexDocumentInput {
            uri: "/repositories/2/resources/19".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/19",
                "title": "Unicode Resource",
                "publish": true,
                "genre": "korespondencja_rękopiśmienna",
                "ark_name": {
                    "current": "https://example.org/ark:/12345/bók-123_żółw",
                    "previous": []
                }
            }),
            enrich: true,
            dynamic_enum_fields: vec!["genre".to_string()],
            arks_enabled: true,
            ..Default::default()
        };

        let doc = build_index_document(input_doc).unwrap();
        assert_eq!(doc["genre_enum_s"], json!(["korespondencja_rękopiśmienna"]));
        assert_eq!(doc["ark_name"], json!(["ark:/12345/bók-123_żółw"]));
    }

    #[test]
    fn omits_arks_when_disabled_or_absent() {
        // ARKs disabled: ark_name omitted even when present in record
        let disabled_input = IndexDocumentInput {
            uri: "/repositories/2/resources/20".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/20",
                "title": "Disabled ARKs Resource",
                "publish": true,
                "ark_name": {
                    "current": "ark:/12345/should_be_omitted",
                    "previous": []
                }
            }),
            enrich: true,
            arks_enabled: false,
            ..Default::default()
        };
        let disabled_doc = build_index_document(disabled_input).unwrap();
        assert!(disabled_doc.get("ark_name").is_none());

        // ARKs enabled but record has no ark_name
        let absent_input = IndexDocumentInput {
            uri: "/repositories/2/resources/21".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/21",
                "title": "No ARK Resource",
                "publish": true
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        let absent_doc = build_index_document(absent_input).unwrap();
        assert!(absent_doc.get("ark_name").is_none());
    }

    #[test]
    fn rejects_malformed_ark_inputs() {
        // Non-object ark_name
        let bad_ark_obj = IndexDocumentInput {
            uri: "/repositories/2/resources/22".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/22",
                "title": "Bad ARK",
                "publish": true,
                "ark_name": "not an object"
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        assert!(build_index_document(bad_ark_obj).is_err());

        // Missing previous array
        let missing_prev = IndexDocumentInput {
            uri: "/repositories/2/resources/23".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/23",
                "title": "Missing Previous ARK",
                "publish": true,
                "ark_name": { "current": "ark:/123" }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        assert!(build_index_document(missing_prev).is_err());

        // Non-array previous
        let non_array_prev = IndexDocumentInput {
            uri: "/repositories/2/resources/24".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/24",
                "title": "Non-array Previous ARK",
                "publish": true,
                "ark_name": { "previous": "not an array" }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        assert!(build_index_document(non_array_prev).is_err());

        // Non-string in previous
        let non_string_prev = IndexDocumentInput {
            uri: "/repositories/2/resources/25".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/25",
                "title": "Non-string in Previous",
                "publish": true,
                "ark_name": { "previous": [123] }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        assert!(build_index_document(non_string_prev).is_err());

        // Non-string current
        let non_string_curr = IndexDocumentInput {
            uri: "/repositories/2/resources/26".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/26",
                "title": "Non-string Current",
                "publish": true,
                "ark_name": { "current": 123, "previous": [] }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };
        assert!(build_index_document(non_string_curr).is_err());
    }

    #[test]
    fn unenriched_requests_do_not_project_enums_or_arks_or_marker() {
        let unenriched = IndexDocumentInput {
            uri: "/repositories/2/resources/27".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/27",
                "title": "Unenriched",
                "publish": true,
                "rights_type": "copyright",
                "ark_name": { "current": "ark:/123", "previous": [] }
            }),
            enrich: false,
            dynamic_enum_fields: vec!["rights_type".to_string()],
            arks_enabled: true,
            ..Default::default()
        };

        let doc = build_index_document(unenriched).unwrap();
        assert!(doc.get("rights_type_enum_s").is_none());
        assert!(doc.get("ark_name").is_none());
        assert!(doc.get("_archivalist_rust_fields").is_none());
    }

    #[test]
    fn test_trim_ark_value_ruby_newline_semantics() {
        // No newline: trims to last ark:
        assert_eq!(trim_ark_value("ark:/12345/foo"), "ark:/12345/foo");
        assert_eq!(
            trim_ark_value("http://example.org/ark:/12345/foo"),
            "ark:/12345/foo"
        );
        assert_eq!(
            trim_ark_value("prefix ark:/first ark:/second"),
            "ark:/second"
        );
        assert_eq!(trim_ark_value("no ark here"), "no ark here");
        assert_eq!(trim_ark_value(""), "");

        // Newline cases: regex /\A.*ark:/ cannot cross \n
        // prefix\nark:/later is unchanged because ark: is not on the first line
        assert_eq!(trim_ark_value("prefix\nark:/later"), "prefix\nark:/later");
        // pre ark:/first\nark:/second trims first line to last ark: on first line and preserves rest
        assert_eq!(
            trim_ark_value("pre ark:/first\nark:/second"),
            "ark:/first\nark:/second"
        );
        // multiple arks on first line with newline suffix
        assert_eq!(
            trim_ark_value("pre ark:/one ark:/two\nark:/three\nark:/four"),
            "ark:/two\nark:/three\nark:/four"
        );
        // \r\n: \r is on first line
        assert_eq!(
            trim_ark_value("pre ark:/first\r\nark:/second"),
            "ark:/first\r\nark:/second"
        );
        assert_eq!(
            trim_ark_value("prefix\r\nark:/later"),
            "prefix\r\nark:/later"
        );
        // \nark:/foo: first line is empty, so unchanged
        assert_eq!(trim_ark_value("\nark:/foo"), "\nark:/foo");
        // ark:/foo\n: first line trimmed, \n preserved
        assert_eq!(trim_ark_value("ark:/foo\n"), "ark:/foo\n");
    }

    #[test]
    fn projects_arks_with_newline_cases_matching_ruby_semantics() {
        let input = IndexDocumentInput {
            uri: "/repositories/2/resources/28".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/28",
                "title": "Newline ARKs",
                "publish": true,
                "ark_name": {
                    "current": "prefix\nark:/later",
                    "previous": [
                        "pre ark:/first\nark:/second",
                        "pre ark:/one ark:/two\nark:/three"
                    ]
                }
            }),
            enrich: true,
            arks_enabled: true,
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let arks = doc.get("ark_name").and_then(Value::as_array).unwrap();
        assert_eq!(
            arks,
            &vec![
                json!("prefix\nark:/later"),
                json!("ark:/first\nark:/second"),
                json!("ark:/two\nark:/three"),
            ]
        );
    }

    #[test]
    fn accession_date_ordinary_and_year_parity() {
        let input = IndexDocumentInput {
            uri: "/repositories/2/accessions/10".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/10",
                "display_string": "Accession 2026-10",
                "accession_date": "2026-10-17",
                "title": "Accession 10",
                "id_0": "ACC-10"
            }),
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
        };
        let doc = build_index_document(input).unwrap();
        assert_eq!(doc["accession_date"], "2026-10-17");
        assert_eq!(doc["accession_date_year"], 2026);
        assert!(matches!(doc["accession_date_year"], Value::Number(_)));
        let fullrecord = doc["fullrecord"].as_array().unwrap();
        assert!(!fullrecord
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("unknown")));
        let rust_fields = doc["_archivalist_rust_fields"].as_array().unwrap();
        assert!(rust_fields.contains(&json!("accession_date_metadata")));
    }

    #[test]
    fn accession_date_sentinel_with_unicode_localized_unknown() {
        // Ukrainian "незнаний"
        let input_uk = IndexDocumentInput {
            uri: "/repositories/2/accessions/11".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/11",
                "display_string": "Accession Sentinel UK",
                "accession_date": "9999-12-31",
                "title": "Accession 11",
                "id_0": "ACC-11"
            }),
            enrich: true,
            accession_date_unknown_label: Some("незнаний".to_string()),
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        };
        let doc_uk = build_index_document(input_uk).unwrap();
        assert_eq!(doc_uk["accession_date"], "незнаний");
        assert_eq!(doc_uk["accession_date_year"], 9999);
        let fullrecord_uk = doc_uk["fullrecord"].as_array().unwrap();
        assert_eq!(fullrecord_uk.last(), Some(&json!("незнаний "))); // Exact trailing space

        // Japanese "道の"
        let input_ja = IndexDocumentInput {
            uri: "/repositories/2/accessions/12".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/12",
                "display_string": "Accession Sentinel JA",
                "accession_date": "9999-12-31",
                "title": "Accession 12",
                "id_0": "ACC-12"
            }),
            enrich: true,
            accession_date_unknown_label: Some("道の".to_string()),
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        };
        let doc_ja = build_index_document(input_ja).unwrap();
        assert_eq!(doc_ja["accession_date"], "道の");
        assert_eq!(doc_ja["accession_date_year"], 9999);
        let fullrecord_ja = doc_ja["fullrecord"].as_array().unwrap();
        assert_eq!(fullrecord_ja.last(), Some(&json!("道の "))); // Exact trailing space

        // German "unbekannt"
        let input_de = IndexDocumentInput {
            uri: "/repositories/2/accessions/13".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/13",
                "display_string": "Accession Sentinel DE",
                "accession_date": "9999-12-31",
                "title": "Accession 13",
                "id_0": "ACC-13"
            }),
            enrich: true,
            accession_date_unknown_label: Some("unbekannt".to_string()),
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        };
        let doc_de = build_index_document(input_de).unwrap();
        assert_eq!(doc_de["accession_date"], "unbekannt");
        assert_eq!(doc_de["accession_date_year"], 9999);
        let fullrecord_de = doc_de["fullrecord"].as_array().unwrap();
        assert_eq!(fullrecord_de.last(), Some(&json!("unbekannt ")));

        // Unicode label with intentional surrounding whitespace: preserved byte-for-byte
        let input_ws_label = IndexDocumentInput {
            uri: "/repositories/2/accessions/13b".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/13b",
                "display_string": "Accession Sentinel WS Label",
                "accession_date": "9999-12-31",
                "title": "Accession 13b",
                "id_0": "ACC-13b"
            }),
            enrich: true,
            accession_date_unknown_label: Some("  незнаний  ".to_string()),
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        };
        let doc_ws_label = build_index_document(input_ws_label).unwrap();
        assert_eq!(doc_ws_label["accession_date"], "  незнаний  ");
        assert_eq!(doc_ws_label["accession_date_year"], 9999);
        let fullrecord_ws = doc_ws_label["fullrecord"].as_array().unwrap();
        assert_eq!(fullrecord_ws.last(), Some(&json!("  незнаний   ")));
    }

    #[test]
    fn accession_date_non_canonical_schema_valid_examples() {
        let make_input = |date_str: &str, year: i32| IndexDocumentInput {
            uri: "/repositories/2/accessions/15".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/15",
                "display_string": "Accession Non-canonical",
                "accession_date": date_str,
                "title": "Accession 15",
                "id_0": "ACC-15"
            }),
            enrich: true,
            parsed_accession_date_year: Some(json!(year)),
            ..Default::default()
        };

        // 1. Year 0000: "0000-01-01" -> year 0
        let doc_zero = build_index_document(make_input("0000-01-01", 0)).unwrap();
        assert_eq!(doc_zero["accession_date"], "0000-01-01");
        assert_eq!(doc_zero["accession_date_year"], 0);

        // 2. Single-digit month: "2026-2-03" -> year 2026
        let doc_single_digit = build_index_document(make_input("2026-2-03", 2026)).unwrap();
        assert_eq!(doc_single_digit["accession_date"], "2026-2-03");
        assert_eq!(doc_single_digit["accession_date_year"], 2026);

        // 3. Surrounding whitespace: "  2026-10-17  " -> preserved exactly, year 2026
        let doc_ws = build_index_document(make_input("  2026-10-17  ", 2026)).unwrap();
        assert_eq!(doc_ws["accession_date"], "  2026-10-17  ");
        assert_eq!(doc_ws["accession_date_year"], 2026);

        // 4. Surrounding whitespace on sentinel: " 9999-12-31 " is NOT sentinel
        let doc_ws_sentinel = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/accessions/15b".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/15b",
                "display_string": "Accession Untrimmed Sentinel",
                "accession_date": " 9999-12-31 ",
                "title": "Accession 15b",
                "id_0": "ACC-15b"
            }),
            enrich: true,
            accession_date_unknown_label: Some("Date Unknown".to_string()),
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(doc_ws_sentinel["accession_date"], " 9999-12-31 ");
        assert_eq!(doc_ws_sentinel["accession_date_year"], 9999);
        let fullrecord = doc_ws_sentinel["fullrecord"].as_array().unwrap();
        assert!(!fullrecord
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("Unknown")));
    }

    #[test]
    fn accession_date_missing_null_and_wrong_type_rejected() {
        let base_record = json!({
            "uri": "/repositories/2/accessions/14",
            "display_string": "Accession Error",
            "title": "Accession 14",
            "id_0": "ACC-14"
        });

        // 1. Missing accession_date
        let missing = IndexDocumentInput {
            uri: "/repositories/2/accessions/14".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: base_record.clone(),
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
        };
        assert!(build_index_document(missing).is_err());

        // 2. Null accession_date
        let mut null_rec = base_record.clone();
        null_rec["accession_date"] = Value::Null;
        let null_input = IndexDocumentInput {
            uri: "/repositories/2/accessions/14".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: null_rec,
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
        };
        assert!(build_index_document(null_input).is_err());

        // 3. Integer accession_date
        let mut int_rec = base_record.clone();
        int_rec["accession_date"] = json!(20261017);
        let int_input = IndexDocumentInput {
            uri: "/repositories/2/accessions/14".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: int_rec,
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
        };
        assert!(build_index_document(int_input).is_err());

        // 4. Boolean accession_date
        let mut bool_rec = base_record.clone();
        bool_rec["accession_date"] = json!(true);
        let bool_input = IndexDocumentInput {
            uri: "/repositories/2/accessions/14".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: bool_rec,
            enrich: true,
            parsed_accession_date_year: Some(json!(2026)),
            ..Default::default()
        };
        assert!(build_index_document(bool_input).is_err());
    }

    #[test]
    fn parsed_accession_date_year_validation_and_rejection() {
        let base_input = |year_val: Option<Value>| IndexDocumentInput {
            uri: "/repositories/2/accessions/14b".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/14b",
                "display_string": "Year Validation",
                "accession_date": "2026-10-17",
                "title": "Accession 14b",
                "id_0": "ACC-14b"
            }),
            enrich: true,
            parsed_accession_date_year: year_val,
            ..Default::default()
        };

        // 1. Missing parsed_accession_date_year
        assert!(build_index_document(base_input(None)).is_err());

        // 2. String parsed_accession_date_year
        assert!(build_index_document(base_input(Some(json!("2026")))).is_err());

        // 3. Float parsed_accession_date_year
        assert!(build_index_document(base_input(Some(json!(2026.5)))).is_err());

        // 4. Boolean parsed_accession_date_year
        assert!(build_index_document(base_input(Some(json!(true)))).is_err());

        // 5. Out of range positive (> i32::MAX)
        assert!(build_index_document(base_input(Some(json!((i32::MAX as i64) + 1)))).is_err());

        // 6. Out of range negative (< i32::MIN)
        assert!(build_index_document(base_input(Some(json!((i32::MIN as i64) - 1)))).is_err());

        // 7. Valid negative year (e.g. BCE)
        let doc_neg = build_index_document(base_input(Some(json!(-44)))).unwrap();
        assert_eq!(doc_neg["accession_date_year"], -44);

        // Sentinel without label
        let sentinel_no_label = IndexDocumentInput {
            uri: "/repositories/2/accessions/15".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/15",
                "display_string": "Sentinel Missing Label",
                "accession_date": "9999-12-31",
                "title": "Accession 15",
                "id_0": "ACC-15"
            }),
            enrich: true,
            accession_date_unknown_label: None,
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        };
        assert!(build_index_document(sentinel_no_label).is_err());

        // Sentinel with empty localized label
        let sentinel_empty_label = IndexDocumentInput {
            uri: "/repositories/2/accessions/15".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/15",
                "display_string": "Sentinel Empty Label",
                "accession_date": "9999-12-31",
                "title": "Accession 15",
                "id_0": "ACC-15"
            }),
            enrich: true,
            accession_date_unknown_label: Some("".to_string()),
            parsed_accession_date_year: Some(json!(9999)),
            ..Default::default()
        };
        assert!(build_index_document(sentinel_empty_label).is_err());
    }

    #[test]
    fn accession_date_non_accession_and_unenriched_isolation() {
        // 1. Unenriched accession request: no accession date fields, no accession_date_metadata marker
        let unenriched_acc = IndexDocumentInput {
            uri: "/repositories/2/accessions/16".to_string(),
            record_type: "accession".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/accessions/16",
                "display_string": "Unenriched Accession",
                "accession_date": "2026-10-17",
                "title": "Accession 16",
                "id_0": "ACC-16"
            }),
            enrich: false,
            ..Default::default()
        };
        let doc_un = build_index_document(unenriched_acc).unwrap();
        assert!(doc_un.get("accession_date").is_none());
        assert!(doc_un.get("accession_date_year").is_none());
        assert!(doc_un.get("_archivalist_rust_fields").is_none());

        // 2. Enriched resource request: no accession_date_metadata marker or accession date fields
        let enriched_res = IndexDocumentInput {
            uri: "/repositories/2/resources/16".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/16",
                "title": "Resource 16",
                "id_0": "RES-16",
                "level": "collection"
            }),
            enrich: true,
            ..Default::default()
        };
        let doc_res = build_index_document(enriched_res).unwrap();
        assert!(doc_res.get("accession_date").is_none());
        assert!(doc_res.get("accession_date_year").is_none());
        let res_markers = doc_res["_archivalist_rust_fields"].as_array().unwrap();
        assert!(!res_markers.contains(&json!("accession_date_metadata")));
    }

    #[test]
    fn pui_final_resource_and_archival_object_privacy_clean() {
        // Resource candidate with nested ancestors, unpublished notes, multiple instances with internal_note and profile notes, unicode
        let resource_candidate = json!({
            "uri": "/repositories/2/resources/50",
            "jsonmodel_type": "resource",
            "title": "Public Collection: Ramón Gómez de la Serna — Obras Completas 📚",
            "id_0": "MS-50",
            "publish": true,
            "suppressed": false,
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/999",
                    "level": "collection",
                    "_resolved": {
                        "title": "SUPER_SECRET_ANCESTOR_COLLECTION",
                        "id_0": "HIDDEN-ANCESTOR",
                        "publish": false,
                        "notes": [{"type": "scopecontent", "publish": true, "subnotes": [{"type": "text", "content": "SECRET_ANCESTOR_NOTE_CONTENT"}]}]
                    }
                }
            ],
            "notes": [
                {
                    "jsonmodel_type": "note_multipart",
                    "type": "scopecontent",
                    "publish": true,
                    "subnotes": [
                        {
                            "jsonmodel_type": "note_text",
                            "content": "Public Scopecontent note for Ramón Gómez in Madrid & Buenos Aires.",
                            "publish": true
                        }
                    ]
                },
                {
                    "jsonmodel_type": "note_singlepart",
                    "type": "abstract",
                    "publish": false,
                    "content": ["UNPUBLISHED_PRIVATE_ABSTRACT_MUST_NEVER_APPEAR_IN_FULLRECORD_PUBLISHED"]
                }
            ],
            "extents": [
                {
                    "number": "5.5",
                    "extent_type": "linear_feet",
                    "portion": "whole",
                    "publish": true
                },
                {
                    "number": "99",
                    "extent_type": "cassettes",
                    "portion": "part",
                    "publish": false
                }
            ],
            "instances": [
                {
                    "instance_type": "mixed_materials",
                    "sub_container": {
                        "top_container": {
                            "ref": "/repositories/2/top_containers/101",
                            "indicator": "Box 1 — Ramón",
                            "_resolved": {
                                "uri": "/repositories/2/top_containers/101",
                                "barcode": "BC-101-MADRID",
                                "internal_note": "TOP_SECRET_CONTAINER_INTERNAL_NOTE_101",
                                "container_profile": {
                                    "_resolved": {
                                        "name": "Standard Hollinger Box",
                                        "notes": "TOP_SECRET_PROFILE_INTERNAL_NOTE_HOL101"
                                    }
                                }
                            }
                        }
                    }
                },
                {
                    "instance_type": "graphic_materials",
                    "sub_container": {
                        "top_container": {
                            "ref": "/repositories/2/top_containers/102",
                            "_resolved": {
                                "uri": "/repositories/2/top_containers/102",
                                "barcode": "BC-102-BA"
                                // Missing internal_note and missing container_profile branch (empty/missing branch check)
                            }
                        }
                    }
                },
                {
                    "instance_type": "audio",
                    // Missing sub_container branch completely
                }
            ]
        });

        let input_res = IndexDocumentInput {
            uri: "/repositories/2/resources/50".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/50",
                "jsonmodel_type": "resource",
                "title": "Public Collection: Ramón Gómez de la Serna — Obras Completas 📚",
                "publish": true
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(resource_candidate),
            ..Default::default()
        };

        let doc_res = build_index_document(input_res).unwrap();

        // 1. Must NOT emit unpublished fullrecord
        assert!(doc_res.get("fullrecord").is_none());

        // 2. Base envelope check
        assert_eq!(doc_res["id"], "/repositories/2/resources/50");
        assert_eq!(doc_res["primary_type"], "resource");

        // 3. Markers
        let markers = doc_res["_archivalist_rust_fields"].as_array().unwrap();
        assert_eq!(markers.len(), 3);
        assert_eq!(markers[0], json!(PUI_FINAL_DOCUMENT_METADATA_MARKER));
        assert_eq!(markers[1], json!(PUI_ROUTING_METADATA_MARKER));
        assert_eq!(markers[2], json!(PUI_NOTES_METADATA_MARKER));

        // 3b. Routing & Notes projections
        let routing = doc_res[PUI_ROUTING_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(routing.len(), 4);
        assert_eq!(routing["marker"], PUI_ROUTING_METADATA_MARKER);
        assert_eq!(routing["id"], "/repositories/2/resources/50#pui");
        assert_eq!(routing["pui_parent_id"], "/repositories/2/resources/50");
        assert_eq!(
            routing["types"],
            json!(["pui", "pui_resource", "pui_record", "pui_only"])
        );

        let notes_proj = doc_res[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(notes_proj.len(), 2);
        assert_eq!(notes_proj["marker"], PUI_NOTES_METADATA_MARKER);
        assert_eq!(notes_proj["notes_published"], Value::Null);

        // 4. PUI Final payload
        let pui_final = doc_res[PUI_FINAL_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(pui_final["marker"], PUI_FINAL_DOCUMENT_METADATA_MARKER);

        let clean_json_str = pui_final["json"].as_str().unwrap();
        let clean_rec: Value = serde_json::from_str(clean_json_str).unwrap();
        let clean_obj = clean_rec.as_object().unwrap();

        // 5. Assert absence of ancestors in final JSON
        assert!(clean_obj.get("ancestors").is_none());

        // 6. Assert absence of top-container internal_note and container_profile notes in final JSON
        let instances = clean_obj["instances"].as_array().unwrap();
        assert_eq!(instances.len(), 3);
        let tc1_resolved = instances[0]["sub_container"]["top_container"]["_resolved"]
            .as_object()
            .unwrap();
        assert!(tc1_resolved.get("internal_note").is_none());
        let cp1_resolved = tc1_resolved["container_profile"]["_resolved"]
            .as_object()
            .unwrap();
        assert!(cp1_resolved.get("notes").is_none());
        assert_eq!(cp1_resolved["name"], "Standard Hollinger Box");
        assert_eq!(tc1_resolved["barcode"], "BC-101-MADRID");

        // 7. Assert absence of any private secrets across entire clean JSON string
        assert!(!clean_json_str.contains("SUPER_SECRET_ANCESTOR_COLLECTION"));
        assert!(!clean_json_str.contains("SECRET_ANCESTOR_NOTE_CONTENT"));
        assert!(!clean_json_str.contains("TOP_SECRET_CONTAINER_INTERNAL_NOTE_101"));
        assert!(!clean_json_str.contains("TOP_SECRET_PROFILE_INTERNAL_NOTE_HOL101"));

        // 8. Assert fullrecord_published content and strict exclusion of private text
        let fullrecord_published = pui_final["fullrecord_published"].as_array().unwrap();
        let published_strings: Vec<&str> = fullrecord_published
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();

        // Must contain published strings
        assert!(published_strings
            .contains(&"Public Collection: Ramón Gómez de la Serna — Obras Completas 📚"));
        assert!(published_strings
            .contains(&"Public Scopecontent note for Ramón Gómez in Madrid & Buenos Aires."));
        assert!(published_strings.contains(&"5.5"));

        // Must NEVER contain unpublished / private strings
        assert!(!published_strings
            .contains(&"UNPUBLISHED_PRIVATE_ABSTRACT_MUST_NEVER_APPEAR_IN_FULLRECORD_PUBLISHED"));
        assert!(!published_strings.contains(&"99"));
        assert!(!published_strings.contains(&"SUPER_SECRET_ANCESTOR_COLLECTION"));
        assert!(!published_strings.contains(&"SECRET_ANCESTOR_NOTE_CONTENT"));
        assert!(!published_strings.contains(&"TOP_SECRET_CONTAINER_INTERNAL_NOTE_101"));
        assert!(!published_strings.contains(&"TOP_SECRET_PROFILE_INTERNAL_NOTE_HOL101"));

        // Archival object candidate check
        let ao_candidate = json!({
            "uri": "/repositories/2/archival_objects/501",
            "jsonmodel_type": "archival_object",
            "title": "Actas Capitulares: Éire & Dublin 1916",
            "component_id": "ITEM-501",
            "publish": true,
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/50",
                    "level": "collection",
                    "_resolved": {
                        "title": "ANCESTOR_RESOURCE_TITLE_PRIVATE",
                        "publish": true
                    }
                }
            ],
            "instances": [
                {
                    "sub_container": {
                        "top_container": {
                            "ref": "/repositories/2/top_containers/202",
                            "_resolved": {
                                "internal_note": "AO_INTERNAL_NOTE_SECRET",
                                "container_profile": {
                                    "_resolved": {
                                        "notes": "AO_PROFILE_NOTE_SECRET"
                                    }
                                }
                            }
                        }
                    }
                }
            ]
        });

        let input_ao = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/501".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/501",
                "jsonmodel_type": "archival_object",
                "title": "Actas Capitulares: Éire & Dublin 1916",
                "display_string": "ITEM-501: Actas Capitulares: Éire & Dublin 1916",
                "publish": true
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(ao_candidate),
            ..Default::default()
        };

        let doc_ao = build_index_document(input_ao).unwrap();
        assert!(doc_ao.get("fullrecord").is_none());
        assert_eq!(doc_ao["id"], "/repositories/2/archival_objects/501");
        assert_eq!(doc_ao["primary_type"], "archival_object");

        let routing_ao = doc_ao[PUI_ROUTING_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(routing_ao.len(), 4);
        assert_eq!(routing_ao["marker"], PUI_ROUTING_METADATA_MARKER);
        assert_eq!(routing_ao["id"], "/repositories/2/archival_objects/501#pui");
        assert_eq!(
            routing_ao["pui_parent_id"],
            "/repositories/2/archival_objects/501"
        );
        assert_eq!(
            routing_ao["types"],
            json!(["pui", "pui_archival_object", "pui_record", "pui_only"])
        );

        let pui_ao = doc_ao[PUI_FINAL_DOCUMENTS_KEY].as_object().unwrap();
        let clean_ao_str = pui_ao["json"].as_str().unwrap();
        let clean_ao: Value = serde_json::from_str(clean_ao_str).unwrap();

        assert!(clean_ao.get("ancestors").is_none());
        assert!(!clean_ao_str.contains("ANCESTOR_RESOURCE_TITLE_PRIVATE"));
        assert!(!clean_ao_str.contains("AO_INTERNAL_NOTE_SECRET"));
        assert!(!clean_ao_str.contains("AO_PROFILE_NOTE_SECRET"));

        let ao_pub_strings: Vec<&str> = pui_ao["fullrecord_published"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(ao_pub_strings.contains(&"Actas Capitulares: Éire & Dublin 1916"));
        assert!(ao_pub_strings.contains(&"ITEM-501"));
        assert!(!ao_pub_strings.contains(&"ANCESTOR_RESOURCE_TITLE_PRIVATE"));
        assert!(!ao_pub_strings.contains(&"AO_INTERNAL_NOTE_SECRET"));
        assert!(!ao_pub_strings.contains(&"AO_PROFILE_NOTE_SECRET"));
    }

    #[test]
    fn pui_final_published_values_and_ordering_match_native() {
        let candidate = json!({
            "uri": "/repositories/2/resources/60",
            "jsonmodel_type": "resource",
            "title": "Alpha Title",
            "finding_aid_title": "Beta Finding Aid",
            "id_0": "Gamma Identifier",
            "publish": true,
            "ancestors": [{"ref": "/repositories/2/resources/1", "title": "Secret"}],
            "instances": [
                {
                    "sub_container": {
                        "top_container": {
                            "ref": "/repositories/2/top_containers/1",
                            "_resolved": {
                                "indicator": "Delta Box 1",
                                "internal_note": "Epsilon Secret"
                            }
                        }
                    }
                }
            ],
            "notes": [
                {
                    "jsonmodel_type": "note_multipart",
                    "type": "scopecontent",
                    "publish": true,
                    "subnotes": [{"type": "text", "content": "Zeta Note Content", "publish": true}]
                }
            ]
        });

        let mut expected_clean = candidate.clone();
        expected_clean
            .as_object_mut()
            .unwrap()
            .shift_remove("ancestors");
        expected_clean["instances"][0]["sub_container"]["top_container"]["_resolved"]
            .as_object_mut()
            .unwrap()
            .shift_remove("internal_note");

        let (expected_published, _expected_unpublished) = extract_string_values(&expected_clean);

        let input = IndexDocumentInput {
            uri: "/repositories/2/resources/60".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/60",
                "jsonmodel_type": "resource",
                "title": "Alpha Title",
                "publish": true
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let pui_final = &doc[PUI_FINAL_DOCUMENTS_KEY];
        let rust_published: Vec<String> = pui_final["fullrecord_published"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        // Exact array ordering and value match
        assert_eq!(rust_published, expected_published);
    }

    #[test]
    fn pui_final_unenriched_and_flag_disabled_isolation() {
        let input = IndexDocumentInput {
            uri: "/repositories/2/resources/70".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/70",
                "title": "Unenriched Resource",
                "publish": true
            }),
            prepare_pui_final: false,
            pui_candidate_record: None,
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        assert!(doc.get("_archivalist_pui_final").is_none());
        assert!(doc.get("_archivalist_rust_fields").is_none());
        assert!(doc.get("fullrecord_published").is_none());
    }

    #[test]
    fn pui_final_rejects_enrichment_and_invalid_candidates() {
        // 1. Mixing enrich (staff) and prepare_pui_final (pui)
        let mixed_input = IndexDocumentInput {
            uri: "/repositories/2/resources/80".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/80", "title": "Mixed"}),
            enrich: true,
            prepare_pui_final: true,
            pui_candidate_record: Some(
                json!({"uri": "/repositories/2/resources/80", "jsonmodel_type": "resource"}),
            ),
            ..Default::default()
        };
        let err = build_index_document(mixed_input).unwrap_err();
        assert!(err.to_string().contains("cannot combine enrich"));

        // 2. Missing pui_candidate_record
        let missing_candidate = IndexDocumentInput {
            uri: "/repositories/2/resources/81".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/81", "title": "Missing Cand"}),
            prepare_pui_final: true,
            pui_candidate_record: None,
            ..Default::default()
        };
        let err2 = build_index_document(missing_candidate).unwrap_err();
        assert!(err2
            .to_string()
            .contains("pui_candidate_record is required"));

        // 3. Non-object pui_candidate_record
        let non_obj_candidate = IndexDocumentInput {
            uri: "/repositories/2/resources/82".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/82", "title": "Array Cand"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!(["invalid", "array"])),
            ..Default::default()
        };
        let err3 = build_index_document(non_obj_candidate).unwrap_err();
        assert!(err3
            .to_string()
            .contains("pui_candidate_record must be a JSON object"));

        // 4. Mismatched candidate URI
        let uri_mismatch = IndexDocumentInput {
            uri: "/repositories/2/resources/83".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/83", "title": "URI Mismatch"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/resources/999_DIFFERENT",
                "jsonmodel_type": "resource"
            })),
            ..Default::default()
        };
        let err4 = build_index_document(uri_mismatch).unwrap_err();
        assert!(err4.to_string().contains("does not match wrapper uri"));

        // 5. Mismatched candidate jsonmodel_type
        let type_mismatch = IndexDocumentInput {
            uri: "/repositories/2/resources/84".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/84", "title": "Type Mismatch"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/resources/84",
                "jsonmodel_type": "archival_object"
            })),
            ..Default::default()
        };
        let err5 = build_index_document(type_mismatch).unwrap_err();
        assert!(err5
            .to_string()
            .contains("does not match wrapper record_type"));

        // 6. Missing candidate uri
        let missing_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/85".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/85", "title": "Missing URI"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "jsonmodel_type": "resource"
            })),
            ..Default::default()
        };
        let err6 = build_index_document(missing_uri).unwrap_err();
        assert!(err6
            .to_string()
            .contains("pui_candidate_record.uri must be a string"));

        // 7. Null candidate uri
        let null_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/86".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/86", "title": "Null URI"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": null,
                "jsonmodel_type": "resource"
            })),
            ..Default::default()
        };
        let err7 = build_index_document(null_uri).unwrap_err();
        assert!(err7
            .to_string()
            .contains("pui_candidate_record.uri must be a string"));

        // 8. Wrong-type (integer) candidate uri
        let int_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/87".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/87", "title": "Int URI"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": 87,
                "jsonmodel_type": "resource"
            })),
            ..Default::default()
        };
        let err8 = build_index_document(int_uri).unwrap_err();
        assert!(err8
            .to_string()
            .contains("pui_candidate_record.uri must be a string"));

        // 9. Missing candidate jsonmodel_type
        let missing_type = IndexDocumentInput {
            uri: "/repositories/2/resources/88".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/88", "title": "Missing Type"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/resources/88"
            })),
            ..Default::default()
        };
        let err9 = build_index_document(missing_type).unwrap_err();
        assert!(err9
            .to_string()
            .contains("pui_candidate_record.jsonmodel_type must be a string"));

        // 10. Null candidate jsonmodel_type
        let null_type = IndexDocumentInput {
            uri: "/repositories/2/resources/89".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/89", "title": "Null Type"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/resources/89",
                "jsonmodel_type": null
            })),
            ..Default::default()
        };
        let err10 = build_index_document(null_type).unwrap_err();
        assert!(err10
            .to_string()
            .contains("pui_candidate_record.jsonmodel_type must be a string"));

        // 11. Wrong-type (array) candidate jsonmodel_type
        let array_type = IndexDocumentInput {
            uri: "/repositories/2/resources/90".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/90", "title": "Array Type"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/resources/90",
                "jsonmodel_type": ["resource"]
            })),
            ..Default::default()
        };
        let err11 = build_index_document(array_type).unwrap_err();
        assert!(err11
            .to_string()
            .contains("pui_candidate_record.jsonmodel_type must be a string"));
    }

    #[test]
    fn pui_routing_and_notes_resource_and_archival_object() {
        let res_candidate = json!({
            "uri": "/repositories/2/resources/101",
            "jsonmodel_type": "resource",
            "title": "Resource Title",
            "publish": true
        });
        let res_input = IndexDocumentInput {
            uri: "/repositories/2/resources/101".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/101",
                "jsonmodel_type": "resource",
                "title": "Resource Title",
                "publish": true,
                "notes": [
                    {
                        "jsonmodel_type": "note_singlepart",
                        "type": "abstract",
                        "publish": true,
                        "content": ["Published resource abstract."]
                    }
                ]
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(res_candidate),
            ..Default::default()
        };

        let res_doc = build_index_document(res_input).unwrap();
        // Envelope check: top-level id and types must NOT be mutated
        assert_eq!(res_doc["id"], "/repositories/2/resources/101");
        assert_eq!(res_doc["types"], json!(["resource"]));

        let res_routing = res_doc[PUI_ROUTING_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(res_routing.len(), 4);
        assert_eq!(res_routing["marker"], PUI_ROUTING_METADATA_MARKER);
        assert_eq!(res_routing["id"], "/repositories/2/resources/101#pui");
        assert_eq!(
            res_routing["pui_parent_id"],
            "/repositories/2/resources/101"
        );
        assert_eq!(
            res_routing["types"],
            json!(["pui", "pui_resource", "pui_record", "pui_only"])
        );

        let res_notes = res_doc[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(res_notes.len(), 2);
        assert_eq!(res_notes["marker"], PUI_NOTES_METADATA_MARKER);
        assert_eq!(
            res_notes["notes_published"],
            json!(["abstract", "Published resource abstract."])
        );

        // Archival object test
        let ao_candidate = json!({
            "uri": "/repositories/2/archival_objects/202",
            "jsonmodel_type": "archival_object",
            "title": "AO Title",
            "publish": true
        });
        let ao_input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/202".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/202",
                "jsonmodel_type": "archival_object",
                "title": "AO Title",
                "publish": true,
                "notes": [
                    {
                        "jsonmodel_type": "note_singlepart",
                        "type": "materialspec",
                        "publish": true,
                        "content": ["Parchment manuscript with wax seal."]
                    }
                ]
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(ao_candidate),
            ..Default::default()
        };

        let ao_doc = build_index_document(ao_input).unwrap();
        assert_eq!(ao_doc["id"], "/repositories/2/archival_objects/202");
        assert_eq!(ao_doc["types"], json!(["archival_object"]));

        let ao_routing = ao_doc[PUI_ROUTING_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(ao_routing.len(), 4);
        assert_eq!(ao_routing["marker"], PUI_ROUTING_METADATA_MARKER);
        assert_eq!(ao_routing["id"], "/repositories/2/archival_objects/202#pui");
        assert_eq!(
            ao_routing["pui_parent_id"],
            "/repositories/2/archival_objects/202"
        );
        assert_eq!(
            ao_routing["types"],
            json!(["pui", "pui_archival_object", "pui_record", "pui_only"])
        );

        let ao_notes = ao_doc[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(ao_notes.len(), 2);
        assert_eq!(ao_notes["marker"], PUI_NOTES_METADATA_MARKER);
        assert_eq!(
            ao_notes["notes_published"],
            json!(["materialspec", "Parchment manuscript with wax seal."])
        );
    }

    #[test]
    fn pui_routing_and_notes_unicode_and_nested_notes() {
        let candidate = json!({
            "uri": "/repositories/2/resources/303",
            "jsonmodel_type": "resource",
            "title": "Úrchlar: Ramón Gómez de la Serna — Obras Completas 📚",
            "publish": true
        });
        let input = IndexDocumentInput {
            uri: "/repositories/2/resources/303".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/303",
                "jsonmodel_type": "resource",
                "title": "Úrchlar: Ramón Gómez de la Serna — Obras Completas 📚",
                "publish": true,
                "notes": [
                    {
                        "jsonmodel_type": "note_multipart",
                        "type": "scopecontent",
                        "publish": true,
                        "subnotes": [
                            {
                                "jsonmodel_type": "note_text",
                                "content": "Café de Pombo (Madrid) — 1920: Tertulia literaria y vanguardia artística ☕🎭",
                                "publish": true
                            },
                            {
                                "jsonmodel_type": "note_text",
                                "content": "UNPUBLISHED_SECRET_NOTE_PORTION: NOT FOR PUBLIC VIEW",
                                "publish": false
                            }
                        ]
                    },
                    {
                        "jsonmodel_type": "note_multipart",
                        "type": "bioghist",
                        "publish": false,
                        "subnotes": [
                            {
                                "jsonmodel_type": "note_text",
                                "content": "ENTIRE_NOTE_IS_UNPUBLISHED_AND_MUST_BE_EXCLUDED",
                                "publish": true
                            }
                        ]
                    },
                    {
                        "jsonmodel_type": "note_singlepart",
                        "type": "general",
                        "publish": true,
                        "content": ["Buenos Aires — Exilio y memorias (1936–1963) 🇦🇷"]
                    }
                ]
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let notes_proj = doc[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        let published_notes: Vec<String> = notes_proj["notes_published"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert_eq!(
            published_notes,
            vec![
                "general".to_string(),
                "Buenos Aires — Exilio y memorias (1936–1963) 🇦🇷".to_string(),
                "scopecontent".to_string(),
                "Café de Pombo (Madrid) — 1920: Tertulia literaria y vanguardia artística ☕🎭"
                    .to_string(),
            ]
        );
        assert!(!published_notes.iter().any(|s| s.contains("UNPUBLISHED")));
    }

    #[test]
    fn pui_routing_and_notes_absent_notes_behavior() {
        let candidate = json!({
            "uri": "/repositories/2/resources/404",
            "jsonmodel_type": "resource",
            "title": "Absence Test",
            "publish": true
        });

        // Case 1: notes field completely omitted
        let input_missing = IndexDocumentInput {
            uri: "/repositories/2/resources/404".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/404",
                "jsonmodel_type": "resource",
                "title": "Absence Test",
                "publish": true
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate.clone()),
            ..Default::default()
        };
        let doc_missing = build_index_document(input_missing).unwrap();
        let notes_missing = doc_missing[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(notes_missing["notes_published"], Value::Null);

        // Case 2: notes field is explicitly null
        let input_null = IndexDocumentInput {
            uri: "/repositories/2/resources/404".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/404",
                "jsonmodel_type": "resource",
                "title": "Absence Test",
                "publish": true,
                "notes": null
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate.clone()),
            ..Default::default()
        };
        let doc_null = build_index_document(input_null).unwrap();
        let notes_null = doc_null[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(notes_null["notes_published"], Value::Null);

        // Case 3: notes field is explicitly false
        let input_false = IndexDocumentInput {
            uri: "/repositories/2/resources/404".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/404",
                "jsonmodel_type": "resource",
                "title": "Absence Test",
                "publish": true,
                "notes": false
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate.clone()),
            ..Default::default()
        };
        let doc_false = build_index_document(input_false).unwrap();
        let notes_false = doc_false[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(notes_false["notes_published"], Value::Null);

        // Case 4: notes field is empty array -> notes_published must be []
        let input_empty = IndexDocumentInput {
            uri: "/repositories/2/resources/404".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/404",
                "jsonmodel_type": "resource",
                "title": "Absence Test",
                "publish": true,
                "notes": []
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate),
            ..Default::default()
        };
        let doc_empty = build_index_document(input_empty).unwrap();
        let notes_empty = doc_empty[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(notes_empty["notes_published"], json!([]));
    }

    #[test]
    fn pui_identity_malformed_record_inputs() {
        let valid_cand = json!({
            "uri": "/repositories/2/resources/505",
            "jsonmodel_type": "resource"
        });

        // 1. Non-object record
        let non_obj_rec = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!("string_record_not_object"),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err1 = build_index_document(non_obj_rec).unwrap_err();
        assert!(err1.to_string().contains("record must be a JSON object"));

        // 2. Missing record uri
        let missing_rec_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"jsonmodel_type": "resource"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err2 = build_index_document(missing_rec_uri).unwrap_err();
        assert!(err2.to_string().contains("record.uri must be a string"));

        // 3. Null record uri
        let null_rec_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": null, "jsonmodel_type": "resource"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err3 = build_index_document(null_rec_uri).unwrap_err();
        assert!(err3.to_string().contains("record.uri must be a string"));

        // 4. Empty record uri
        let empty_rec_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "   ", "jsonmodel_type": "resource"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err4 = build_index_document(empty_rec_uri).unwrap_err();
        assert!(
            err4.to_string().contains("does not match wrapper uri")
                || err4.to_string().contains("record.uri must not be empty")
        );

        // 5. Mismatched record uri
        let mismatch_rec_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/999_MISMATCH",
                "jsonmodel_type": "resource"
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err5 = build_index_document(mismatch_rec_uri).unwrap_err();
        assert!(err5.to_string().contains("does not match wrapper uri"));

        // 6. Missing record jsonmodel_type
        let missing_rec_type = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({"uri": "/repositories/2/resources/505"}),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err6 = build_index_document(missing_rec_type).unwrap_err();
        assert!(err6
            .to_string()
            .contains("record.jsonmodel_type must be a string"));

        // 7. Null record jsonmodel_type
        let null_rec_type = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/505",
                "jsonmodel_type": null
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err7 = build_index_document(null_rec_type).unwrap_err();
        assert!(err7
            .to_string()
            .contains("record.jsonmodel_type must be a string"));

        // 8. Empty record jsonmodel_type
        let empty_rec_type = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/505",
                "jsonmodel_type": " "
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand.clone()),
            ..Default::default()
        };
        let err8 = build_index_document(empty_rec_type).unwrap_err();
        assert!(err8
            .to_string()
            .contains("record.jsonmodel_type must not be empty"));

        // 9. Mismatched record jsonmodel_type
        let mismatch_rec_type = IndexDocumentInput {
            uri: "/repositories/2/resources/505".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/505",
                "jsonmodel_type": "archival_object"
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(valid_cand),
            ..Default::default()
        };
        let err9 = build_index_document(mismatch_rec_type).unwrap_err();
        assert!(err9
            .to_string()
            .contains("does not match wrapper record_type"));
    }

    #[test]
    fn pui_multi_record_batch_isolation() {
        let res_input = IndexDocumentInput {
            uri: "/repositories/2/resources/601".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/601",
                "jsonmodel_type": "resource",
                "title": "Batch Resource 601",
                "publish": true,
                "notes": [
                    {
                        "jsonmodel_type": "note_singlepart",
                        "type": "abstract",
                        "publish": true,
                        "content": ["Res 601 Note"]
                    }
                ]
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/resources/601",
                "jsonmodel_type": "resource",
                "title": "Batch Resource 601",
                "publish": true
            })),
            ..Default::default()
        };

        let ao_input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/602".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/602",
                "jsonmodel_type": "archival_object",
                "title": "Batch AO 602",
                "publish": true,
                "notes": [
                    {
                        "jsonmodel_type": "note_singlepart",
                        "type": "scopecontent",
                        "publish": true,
                        "content": ["AO 602 Note"]
                    }
                ]
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/archival_objects/602",
                "jsonmodel_type": "archival_object",
                "title": "Batch AO 602",
                "publish": true
            })),
            ..Default::default()
        };

        let plain_input = IndexDocumentInput {
            uri: "/repositories/2/resources/603".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/603",
                "jsonmodel_type": "resource",
                "title": "Batch Plain 603",
                "publish": true
            }),
            prepare_pui_final: false,
            pui_candidate_record: None,
            ..Default::default()
        };

        let response = build_index_document_batch(IndexDocumentBatchRequest {
            records: vec![res_input, ao_input, plain_input],
        })
        .unwrap();
        let docs = response.documents;
        assert_eq!(docs.len(), 3);

        // Record 1 (Resource)
        let doc0 = &docs[0];
        assert_eq!(doc0["id"], "/repositories/2/resources/601");
        assert_eq!(doc0["types"], json!(["resource"]));
        let r0 = doc0[PUI_ROUTING_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(r0["id"], "/repositories/2/resources/601#pui");
        assert_eq!(r0["pui_parent_id"], "/repositories/2/resources/601");
        assert_eq!(
            r0["types"],
            json!(["pui", "pui_resource", "pui_record", "pui_only"])
        );
        let n0 = doc0[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(n0["notes_published"], json!(["abstract", "Res 601 Note"]));

        // Record 2 (Archival Object)
        let doc1 = &docs[1];
        assert_eq!(doc1["id"], "/repositories/2/archival_objects/602");
        assert_eq!(doc1["types"], json!(["archival_object"]));
        let r1 = doc1[PUI_ROUTING_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(r1["id"], "/repositories/2/archival_objects/602#pui");
        assert_eq!(r1["pui_parent_id"], "/repositories/2/archival_objects/602");
        assert_eq!(
            r1["types"],
            json!(["pui", "pui_archival_object", "pui_record", "pui_only"])
        );
        let n1 = doc1[PUI_NOTES_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(
            n1["notes_published"],
            json!(["scopecontent", "AO 602 Note"])
        );

        // Record 3 (Unaccelerated/plain)
        let doc2 = &docs[2];
        assert_eq!(doc2["id"], "/repositories/2/resources/603");
        assert_eq!(doc2["types"], json!(["resource"]));
        assert!(doc2.get(PUI_ROUTING_DOCUMENTS_KEY).is_none());
        assert!(doc2.get(PUI_NOTES_DOCUMENTS_KEY).is_none());
        assert!(doc2.get(PUI_FINAL_DOCUMENTS_KEY).is_none());
        assert!(doc2.get("_archivalist_rust_fields").is_none());
    }

    #[test]
    fn test_compute_pui_conditional_additions_all_types() {
        // 1. Resource
        assert_eq!(
            compute_pui_conditional_additions("resource", false),
            vec!["pui_collection", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("resource", true),
            Vec::<String>::new()
        );

        // 2. Classification
        assert_eq!(
            compute_pui_conditional_additions("classification", false),
            vec!["pui_record_group", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("classification", true),
            Vec::<String>::new()
        );

        // 3. Accession
        assert_eq!(
            compute_pui_conditional_additions("accession", false),
            vec!["pui_record", "pui_accession", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("accession", true),
            vec!["pui_record"]
        );

        // 4. Digital object
        assert_eq!(
            compute_pui_conditional_additions("digital_object", false),
            vec!["pui_record", "pui_digital_object", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("digital_object", true),
            vec!["pui_record"]
        );

        // 5. Digital object component
        assert_eq!(
            compute_pui_conditional_additions("digital_object_component", false),
            vec!["pui_record", "pui_digital_object_component", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("digital_object_component", true),
            vec!["pui_record"]
        );

        // 6. Agent person
        assert_eq!(
            compute_pui_conditional_additions("agent_person", false),
            vec!["pui_agent", "pui_person", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("agent_person", true),
            vec!["pui_agent"]
        );

        // 7. Agent corporate entity
        assert_eq!(
            compute_pui_conditional_additions("agent_corporate_entity", false),
            vec!["pui_agent", "pui_agent_corporate_entity", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("agent_corporate_entity", true),
            vec!["pui_agent"]
        );

        // 8. Other primary type (e.g. subject, location)
        assert_eq!(
            compute_pui_conditional_additions("subject", false),
            vec!["pui_subject", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("location", false),
            vec!["pui_location", "pui"]
        );
        assert_eq!(
            compute_pui_conditional_additions("archival_object", true),
            Vec::<String>::new()
        );

        // 9. Top container
        assert_eq!(
            compute_pui_conditional_additions("top_container", false),
            vec!["pui_top_container", "pui"]
        );
    }

    #[test]
    fn test_pui_availability_plan_for_record_types() {
        let types = vec![
            ("resource", vec!["pui_collection", "pui"]),
            ("classification", vec!["pui_record_group", "pui"]),
            ("accession", vec!["pui_record", "pui_accession", "pui"]),
            (
                "digital_object",
                vec!["pui_record", "pui_digital_object", "pui"],
            ),
            (
                "digital_object_component",
                vec!["pui_record", "pui_digital_object_component", "pui"],
            ),
            ("agent_person", vec!["pui_agent", "pui_person", "pui"]),
            (
                "agent_corporate_entity",
                vec!["pui_agent", "pui_agent_corporate_entity", "pui"],
            ),
            ("subject", vec!["pui_subject", "pui"]),
        ];

        for (rec_type, expected_additions) in types {
            let uri = format!("/repositories/2/{rec_type}s/100");
            let input = IndexDocumentInput {
                uri: uri.clone(),
                record_type: rec_type.to_string(),
                repository: json!(2),
                record: json!({
                    "uri": uri,
                    "jsonmodel_type": rec_type,
                    "title": format!("Sample {rec_type}"),
                    "publish": true,
                }),
                prepare_pui_availability: true,
                record_inheritance_has_type: Some(false),
                ..Default::default()
            };

            let doc = build_index_document(input).unwrap();
            let fields = doc["_archivalist_rust_fields"].as_array().unwrap();
            assert!(fields.contains(&Value::String(PUI_AVAILABILITY_METADATA_MARKER.to_string())));

            let plan = doc[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
            assert_eq!(plan["marker"], PUI_AVAILABILITY_METADATA_MARKER);
            assert_eq!(plan["uri"], uri);
            assert_eq!(plan["primary_type"], rec_type);
            assert_eq!(plan["record_inheritance_has_type"], false);
            assert_eq!(plan["conditional_additions"], json!(expected_additions));
            assert!(plan["top_container_override"].is_null());
        }
    }

    #[test]
    fn test_pui_availability_inheritance_backed_type() {
        let uri = "/repositories/2/archival_objects/500".to_string();
        let input = IndexDocumentInput {
            uri: uri.clone(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": uri,
                "jsonmodel_type": "archival_object",
                "title": "Inheritance-backed AO",
                "publish": true,
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(true),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let plan = doc[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(plan["marker"], PUI_AVAILABILITY_METADATA_MARKER);
        assert_eq!(plan["uri"], uri);
        assert_eq!(plan["primary_type"], "archival_object");
        assert_eq!(plan["record_inheritance_has_type"], true);
        assert_eq!(plan["conditional_additions"], json!([]));
        assert!(plan["top_container_override"].is_null());
    }

    #[test]
    fn test_pui_availability_unpublished_records() {
        let uri = "/repositories/2/resources/999".to_string();
        let input = IndexDocumentInput {
            uri: uri.clone(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": uri,
                "jsonmodel_type": "resource",
                "title": "Unpublished resource",
                "publish": false,
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        assert_eq!(doc["publish"], false);

        let plan = doc[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(plan["marker"], PUI_AVAILABILITY_METADATA_MARKER);
        assert_eq!(plan["uri"], uri);
        assert_eq!(plan["primary_type"], "resource");
        assert_eq!(plan["record_inheritance_has_type"], false);
        assert_eq!(
            plan["conditional_additions"],
            json!(["pui_collection", "pui"])
        );
        assert!(plan["top_container_override"].is_null());
    }

    #[test]
    fn test_pui_availability_unicode_type_safe_inputs() {
        let uri = "/repositories/2/resources/colección-josé-📚".to_string();
        let input = IndexDocumentInput {
            uri: uri.clone(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": uri,
                "jsonmodel_type": "resource",
                "title": "Colección José Hernández 📚",
                "publish": true,
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let plan = doc[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(plan["uri"], uri);
        assert_eq!(plan["marker"], PUI_AVAILABILITY_METADATA_MARKER);
        assert_eq!(
            plan["conditional_additions"],
            json!(["pui_collection", "pui"])
        );
    }

    #[test]
    fn test_pui_availability_exhaustive_top_container_matrix() {
        // Matrix:
        // initial_publish (false, true) x is_linked_to_published_record (false, true, null)
        let matrix = vec![
            // (initial_doc_publish, is_linked, expected_tc_publish_in_plan, expected_final_doc_publish, expected_final_types)
            (
                false,
                Some(false),
                Value::Bool(false),
                false,
                vec!["top_container"],
            ),
            (
                false,
                Some(true),
                Value::Bool(true),
                true,
                vec!["top_container", "pui_container", "pui"],
            ),
            (
                true,
                Some(false),
                Value::Bool(false),
                false,
                vec!["top_container", "pui_top_container", "pui"],
            ),
            (
                true,
                Some(true),
                Value::Bool(true),
                true,
                vec![
                    "top_container",
                    "pui_top_container",
                    "pui",
                    "pui_container",
                    "pui",
                ],
            ),
            (
                false,
                None,
                Value::Null,
                false, // nil is falsey
                vec!["top_container"],
            ),
            (
                true,
                None,
                Value::Null,
                false, // nil is falsey
                vec!["top_container", "pui_top_container", "pui"],
            ),
        ];

        for (
            initial_publish,
            is_linked,
            expected_plan_publish,
            expected_final_publish,
            expected_final_types,
        ) in matrix
        {
            let uri = format!(
                "/repositories/2/top_containers/tc_init_{initial_publish}_linked_{:?}",
                is_linked
            );
            let mut rec = json!({
                "uri": uri,
                "jsonmodel_type": "top_container",
                "title": "Box 1",
                "publish": initial_publish,
            });
            if let Some(linked) = is_linked {
                rec["is_linked_to_published_record"] = Value::Bool(linked);
            }

            let input = IndexDocumentInput {
                uri: uri.clone(),
                record_type: "top_container".to_string(),
                repository: json!(2),
                record: rec,
                prepare_pui_availability: true,
                record_inheritance_has_type: Some(false),
                ..Default::default()
            };

            let doc = build_index_document(input).unwrap();
            assert_eq!(doc["publish"], initial_publish);

            let plan = doc[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
            assert_eq!(plan["marker"], PUI_AVAILABILITY_METADATA_MARKER);
            assert_eq!(plan["uri"], uri);
            assert_eq!(plan["primary_type"], "top_container");
            assert_eq!(plan["record_inheritance_has_type"], false);
            assert_eq!(
                plan["conditional_additions"],
                json!(["pui_top_container", "pui"])
            );

            let tc_override = plan["top_container_override"].as_object().unwrap();
            assert_eq!(tc_override["publish"], expected_plan_publish);
            assert_eq!(tc_override["additions"], json!(["pui_container", "pui"]));

            // Simulate the exact native application flow on the document:
            let mut types = doc["types"].as_array().unwrap().clone();
            let mut current_publish = doc["publish"].clone();

            // Step 1: if doc['publish'] (truthy check)
            if current_publish == Value::Bool(true) {
                for item in plan["conditional_additions"].as_array().unwrap() {
                    types.push(item.clone());
                }
            }

            // Step 2 & 3: Top container override: doc['publish'] = is_linked_to_published_record
            current_publish = tc_override["publish"].clone();
            let is_truthy = !current_publish.is_null() && current_publish != Value::Bool(false);
            if is_truthy {
                for item in tc_override["additions"].as_array().unwrap() {
                    types.push(item.clone());
                }
            }

            assert_eq!(
                is_truthy, expected_final_publish,
                "final publish mismatch for initial={initial_publish}, linked={is_linked:?}"
            );
            assert_eq!(
                types,
                expected_final_types
                    .iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect::<Vec<_>>(),
                "final types mismatch for initial={initial_publish}, linked={is_linked:?}"
            );
        }
    }

    #[test]
    fn test_pui_availability_multi_record_batch_isolation() {
        let res_input = IndexDocumentInput {
            uri: "/repositories/2/resources/701".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/701",
                "jsonmodel_type": "resource",
                "title": "Batch Resource 701",
                "publish": true,
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };

        let tc_input = IndexDocumentInput {
            uri: "/repositories/2/top_containers/702".to_string(),
            record_type: "top_container".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/top_containers/702",
                "jsonmodel_type": "top_container",
                "title": "Box 702",
                "publish": false,
                "is_linked_to_published_record": true,
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };

        let ao_input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/703".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/703",
                "jsonmodel_type": "archival_object",
                "title": "Batch AO 703",
                "publish": true,
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(true),
            ..Default::default()
        };

        let unaccelerated_input = IndexDocumentInput {
            uri: "/repositories/2/resources/704".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/704",
                "jsonmodel_type": "resource",
                "title": "Batch Plain 704",
                "publish": true,
            }),
            prepare_pui_availability: false,
            record_inheritance_has_type: None,
            ..Default::default()
        };

        let response = build_index_document_batch(IndexDocumentBatchRequest {
            records: vec![res_input, tc_input, ao_input, unaccelerated_input],
        })
        .unwrap();

        assert_eq!(response.documents.len(), 4);

        // Doc 0: Resource
        let d0 = &response.documents[0];
        assert_eq!(d0["id"], "/repositories/2/resources/701");
        let p0 = d0[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(p0["marker"], PUI_AVAILABILITY_METADATA_MARKER);
        assert_eq!(p0["uri"], "/repositories/2/resources/701");
        assert_eq!(p0["primary_type"], "resource");
        assert_eq!(
            p0["conditional_additions"],
            json!(["pui_collection", "pui"])
        );
        assert!(p0["top_container_override"].is_null());

        // Doc 1: Top container
        let d1 = &response.documents[1];
        assert_eq!(d1["id"], "/repositories/2/top_containers/702");
        let p1 = d1[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(p1["marker"], PUI_AVAILABILITY_METADATA_MARKER);
        assert_eq!(p1["uri"], "/repositories/2/top_containers/702");
        assert_eq!(p1["primary_type"], "top_container");
        assert_eq!(
            p1["conditional_additions"],
            json!(["pui_top_container", "pui"])
        );
        assert_eq!(
            p1["top_container_override"],
            json!({
                "publish": true,
                "additions": ["pui_container", "pui"]
            })
        );

        // Doc 2: Archival Object with inheritance true
        let d2 = &response.documents[2];
        assert_eq!(d2["id"], "/repositories/2/archival_objects/703");
        let p2 = d2[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(p2["marker"], PUI_AVAILABILITY_METADATA_MARKER);
        assert_eq!(p2["uri"], "/repositories/2/archival_objects/703");
        assert_eq!(p2["primary_type"], "archival_object");
        assert_eq!(p2["record_inheritance_has_type"], true);
        assert_eq!(p2["conditional_additions"], json!([]));
        assert!(p2["top_container_override"].is_null());

        // Doc 3: Unaccelerated record
        let d3 = &response.documents[3];
        assert_eq!(d3["id"], "/repositories/2/resources/704");
        assert!(d3.get(PUI_AVAILABILITY_DOCUMENTS_KEY).is_none());
        assert!(d3.get("_archivalist_rust_fields").is_none());
    }

    #[test]
    fn test_pui_availability_cannot_combine_with_prepare_pui_final() {
        let input = IndexDocumentInput {
            uri: "/repositories/2/resources/801".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/801",
                "jsonmodel_type": "resource",
                "title": "Combined",
            }),
            prepare_pui_final: true,
            prepare_pui_availability: true,
            ..Default::default()
        };

        let err = build_index_document(input).unwrap_err();
        assert!(err
            .to_string()
            .contains("cannot combine prepare_pui_final (pui) and prepare_pui_availability"));
    }

    #[test]
    fn test_pui_availability_record_identity_validation() {
        // 1. Missing URI in record
        let missing_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "jsonmodel_type": "resource",
                "title": "Missing URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_miss_uri = build_index_document(missing_uri).unwrap_err();
        assert!(err_miss_uri.to_string().contains("record.uri is required"));

        // 2. Null URI in record
        let null_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": null,
                "jsonmodel_type": "resource",
                "title": "Null URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_null_uri = build_index_document(null_uri).unwrap_err();
        assert!(err_null_uri
            .to_string()
            .contains("record.uri must be a non-empty string"));

        // 3. Numeric URI in record
        let num_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": 802,
                "jsonmodel_type": "resource",
                "title": "Numeric URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_num_uri = build_index_document(num_uri).unwrap_err();
        assert!(err_num_uri
            .to_string()
            .contains("record.uri must be a non-empty string"));

        // 4. Array URI in record
        let arr_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": ["/repositories/2/resources/802"],
                "jsonmodel_type": "resource",
                "title": "Array URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_arr_uri = build_index_document(arr_uri).unwrap_err();
        assert!(err_arr_uri
            .to_string()
            .contains("record.uri must be a non-empty string"));

        // 5. Hash URI in record
        let hash_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": {"ref": "/repositories/2/resources/802"},
                "jsonmodel_type": "resource",
                "title": "Hash URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_hash_uri = build_index_document(hash_uri).unwrap_err();
        assert!(err_hash_uri
            .to_string()
            .contains("record.uri must be a non-empty string"));

        // 6. Empty string URI in record
        let empty_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "",
                "jsonmodel_type": "resource",
                "title": "Empty URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_empty_uri = build_index_document(empty_uri).unwrap_err();
        assert!(err_empty_uri
            .to_string()
            .contains("record.uri must not be empty"));

        // 7. Whitespace string URI in record
        let ws_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "   ",
                "jsonmodel_type": "resource",
                "title": "Whitespace URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_ws_uri = build_index_document(ws_uri).unwrap_err();
        assert!(err_ws_uri
            .to_string()
            .contains("record.uri must not be empty"));

        // 8. Mismatched URI in record
        let bad_uri = IndexDocumentInput {
            uri: "/repositories/2/resources/802".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/999_MISMATCH",
                "jsonmodel_type": "resource",
                "title": "Bad URI",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_bad_uri = build_index_document(bad_uri).unwrap_err();
        assert!(err_bad_uri
            .to_string()
            .contains("does not match wrapper uri"));

        // 9. Missing jsonmodel_type in record
        let missing_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "title": "Missing Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_miss_type = build_index_document(missing_type).unwrap_err();
        assert!(err_miss_type
            .to_string()
            .contains("record.jsonmodel_type is required"));

        // 10. Null jsonmodel_type in record
        let null_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": null,
                "title": "Null Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_null_type = build_index_document(null_type).unwrap_err();
        assert!(err_null_type
            .to_string()
            .contains("record.jsonmodel_type must be a non-empty string"));

        // 11. Numeric jsonmodel_type in record
        let num_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": 123,
                "title": "Numeric Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_num_type = build_index_document(num_type).unwrap_err();
        assert!(err_num_type
            .to_string()
            .contains("record.jsonmodel_type must be a non-empty string"));

        // 12. Array jsonmodel_type in record
        let arr_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": ["resource"],
                "title": "Array Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_arr_type = build_index_document(arr_type).unwrap_err();
        assert!(err_arr_type
            .to_string()
            .contains("record.jsonmodel_type must be a non-empty string"));

        // 13. Hash jsonmodel_type in record
        let hash_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": {"name": "resource"},
                "title": "Hash Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_hash_type = build_index_document(hash_type).unwrap_err();
        assert!(err_hash_type
            .to_string()
            .contains("record.jsonmodel_type must be a non-empty string"));

        // 14. Empty string jsonmodel_type in record
        let empty_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": "",
                "title": "Empty Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_empty_type = build_index_document(empty_type).unwrap_err();
        assert!(err_empty_type
            .to_string()
            .contains("record.jsonmodel_type must not be empty"));

        // 15. Whitespace string jsonmodel_type in record
        let ws_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": "   ",
                "title": "Whitespace Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_ws_type = build_index_document(ws_type).unwrap_err();
        assert!(err_ws_type
            .to_string()
            .contains("record.jsonmodel_type must not be empty"));

        // 16. Mismatched jsonmodel_type in record
        let bad_type = IndexDocumentInput {
            uri: "/repositories/2/resources/803".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/803",
                "jsonmodel_type": "archival_object",
                "title": "Bad Type",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_bad_type = build_index_document(bad_type).unwrap_err();
        assert!(err_bad_type
            .to_string()
            .contains("does not match wrapper record_type"));

        // 17. Missing record_inheritance_has_type (None) when prepare_pui_availability is true
        let missing_inh = IndexDocumentInput {
            uri: "/repositories/2/resources/804".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/804",
                "jsonmodel_type": "resource",
                "title": "Missing Inheritance",
            }),
            prepare_pui_availability: true,
            record_inheritance_has_type: None,
            ..Default::default()
        };
        let err_miss_inh = build_index_document(missing_inh).unwrap_err();
        assert!(err_miss_inh
            .to_string()
            .contains("record_inheritance_has_type must be explicitly present and boolean"));

        // 18. Non-object direct record
        let non_obj_rec = IndexDocumentInput {
            uri: "/repositories/2/resources/805".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!("string_record"),
            prepare_pui_availability: true,
            record_inheritance_has_type: Some(false),
            ..Default::default()
        };
        let err_non_obj = build_index_document(non_obj_rec).unwrap_err();
        assert!(err_non_obj
            .to_string()
            .contains("record must be a JSON object"));

        // 19. Serde deserialization rejects non-boolean types for record_inheritance_has_type
        let bad_str_json = r#"{
            "uri": "/repositories/2/resources/806",
            "record_type": "resource",
            "repository": 2,
            "prepare_pui_availability": true,
            "record_inheritance_has_type": "true",
            "record": {"uri": "/repositories/2/resources/806", "jsonmodel_type": "resource"}
        }"#;
        assert!(serde_json::from_str::<IndexDocumentInput>(bad_str_json).is_err());

        let bad_int_json = r#"{
            "uri": "/repositories/2/resources/807",
            "record_type": "resource",
            "repository": 2,
            "prepare_pui_availability": true,
            "record_inheritance_has_type": 1,
            "record": {"uri": "/repositories/2/resources/807", "jsonmodel_type": "resource"}
        }"#;
        assert!(serde_json::from_str::<IndexDocumentInput>(bad_int_json).is_err());

        // 20. Serde deserializes null to None, which build_index_document rejects
        let null_inh_json = r#"{
            "uri": "/repositories/2/resources/808",
            "record_type": "resource",
            "repository": 2,
            "prepare_pui_availability": true,
            "record_inheritance_has_type": null,
            "record": {"uri": "/repositories/2/resources/808", "jsonmodel_type": "resource"}
        }"#;
        let decoded_null = serde_json::from_str::<IndexDocumentInput>(null_inh_json).unwrap();
        assert_eq!(decoded_null.record_inheritance_has_type, None);
        assert!(build_index_document(decoded_null).is_err());

        // 21. Serde deserializes omitted field to None, which build_index_document rejects when prepare_pui_availability is true
        let omitted_inh_json = r#"{
            "uri": "/repositories/2/resources/809",
            "record_type": "resource",
            "repository": 2,
            "prepare_pui_availability": true,
            "record": {"uri": "/repositories/2/resources/809", "jsonmodel_type": "resource"}
        }"#;
        let decoded_omitted = serde_json::from_str::<IndexDocumentInput>(omitted_inh_json).unwrap();
        assert_eq!(decoded_omitted.record_inheritance_has_type, None);
        assert!(build_index_document(decoded_omitted).is_err());

        // 22. Backward compatibility: omitted record_inheritance_has_type succeeds when prepare_pui_availability is false
        let non_avail_json = r#"{
            "uri": "/repositories/2/resources/810",
            "record_type": "resource",
            "repository": 2,
            "prepare_pui_availability": false,
            "record": {"uri": "/repositories/2/resources/810", "jsonmodel_type": "resource"}
        }"#;
        let decoded_non_avail = serde_json::from_str::<IndexDocumentInput>(non_avail_json).unwrap();
        assert_eq!(decoded_non_avail.record_inheritance_has_type, None);
        assert!(build_index_document(decoded_non_avail).is_ok());
    }

    #[test]
    fn test_pui_availability_top_container_non_boolean_publish() {
        for val in [
            json!(0),
            json!(1),
            json!("linked_val"),
            json!(null),
            json!(false),
            json!(true),
        ] {
            let tc_input = IndexDocumentInput {
                uri: "/repositories/2/top_containers/901".to_string(),
                record_type: "top_container".to_string(),
                repository: json!(2),
                record: json!({
                    "uri": "/repositories/2/top_containers/901",
                    "jsonmodel_type": "top_container",
                    "title": "Box 901",
                    "publish": true,
                    "is_linked_to_published_record": val,
                }),
                prepare_pui_availability: true,
                record_inheritance_has_type: Some(false),
                ..Default::default()
            };
            let doc = build_index_document(tc_input).unwrap();
            let p = doc[PUI_AVAILABILITY_DOCUMENTS_KEY].as_object().unwrap();
            let tc_override = p["top_container_override"].as_object().unwrap();
            assert_eq!(tc_override["publish"], val);
            assert_eq!(tc_override["additions"], json!(["pui_container", "pui"]));
        }
    }

    #[test]
    fn test_pui_auxiliary_all_kinds_and_shapes() {
        // 1. resource_ordered_records
        let ord_record = json!({
            "uris": ["/repositories/2/archival_objects/1", "/repositories/2/archival_objects/2"]
        });
        let ord_input = IndexDocumentInput {
            uri: "/repositories/2/resources/10/ordered_records".to_string(),
            record_type: "resource_ordered_records".to_string(),
            pui_parent_id: Some("/repositories/2/resources/10".to_string()),
            record: ord_record.clone(),
            pui_auxiliary_json: Some(serde_json::to_string(&ord_record).unwrap()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let ord_doc = build_index_document(ord_input).unwrap();
        assert_eq!(
            ord_doc["id"],
            "/repositories/2/resources/10/ordered_records"
        );
        assert_eq!(
            ord_doc["uri"],
            "/repositories/2/resources/10/ordered_records"
        );
        assert_eq!(ord_doc["pui_parent_id"], "/repositories/2/resources/10");
        assert_eq!(ord_doc["publish"], "true");
        assert!(ord_doc["publish"].is_string());
        assert_ne!(ord_doc["publish"], true);
        assert_eq!(ord_doc["primary_type"], "resource_ordered_records");
        assert_eq!(ord_doc["types"], json!(["pui"]));
        let ord_parsed: Value = serde_json::from_str(ord_doc["json"].as_str().unwrap()).unwrap();
        assert_eq!(
            ord_parsed["uris"],
            json!([
                "/repositories/2/archival_objects/1",
                "/repositories/2/archival_objects/2"
            ])
        );
        let ord_meta = ord_doc[PUI_AUXILIARY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(ord_meta["marker"], PUI_AUXILIARY_METADATA_MARKER);
        assert_eq!(ord_meta["auxiliary_type"], "resource_ordered_records");

        // 2. tree_root
        let root_record = json!({
            "title": "Root Collection",
            "waypoints": 3
        });
        let root_input = IndexDocumentInput {
            uri: "/repositories/2/resources/10/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/10".to_string()),
            record: root_record.clone(),
            pui_auxiliary_json: Some(serde_json::to_string(&root_record).unwrap()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let root_doc = build_index_document(root_input).unwrap();
        assert_eq!(root_doc["id"], "/repositories/2/resources/10/tree/root");
        assert_eq!(root_doc["uri"], "/repositories/2/resources/10/tree/root");
        assert_eq!(root_doc["pui_parent_id"], "/repositories/2/resources/10");
        assert_eq!(root_doc["publish"], "true");
        assert_eq!(root_doc["primary_type"], "tree_root");
        assert_eq!(root_doc["types"], json!(["pui"]));
        let root_meta = root_doc[PUI_AUXILIARY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(root_meta["marker"], PUI_AUXILIARY_METADATA_MARKER);
        assert_eq!(root_meta["auxiliary_type"], "tree_root");

        // 3. tree_waypoint
        let wp_record = json!([
            {"uri": "/repositories/2/archival_objects/1", "child_count": 2},
            {"uri": "/repositories/2/archival_objects/2", "child_count": 0}
        ]);
        let wp_input = IndexDocumentInput {
            uri: "/repositories/2/resources/10/tree/waypoint__0".to_string(),
            record_type: "tree_waypoint".to_string(),
            pui_parent_id: Some("/repositories/2/resources/10".to_string()),
            record: wp_record.clone(),
            pui_auxiliary_json: Some(serde_json::to_string(&wp_record).unwrap()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let wp_doc = build_index_document(wp_input).unwrap();
        assert_eq!(
            wp_doc["id"],
            "/repositories/2/resources/10/tree/waypoint__0"
        );
        assert_eq!(
            wp_doc["uri"],
            "/repositories/2/resources/10/tree/waypoint__0"
        );
        assert_eq!(wp_doc["pui_parent_id"], "/repositories/2/resources/10");
        assert_eq!(wp_doc["publish"], "true");
        assert_eq!(wp_doc["primary_type"], "tree_waypoint");
        assert_eq!(wp_doc["types"], json!(["pui"]));
        let wp_meta = wp_doc[PUI_AUXILIARY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(wp_meta["marker"], PUI_AUXILIARY_METADATA_MARKER);
        assert_eq!(wp_meta["auxiliary_type"], "tree_waypoint");

        // 4. tree_node
        let node_record = json!({
            "uri": "/repositories/2/archival_objects/1",
            "title": "Series 1",
            "waypoints": 1
        });
        let node_input = IndexDocumentInput {
            uri: "/repositories/2/resources/10/tree/node_/repositories/2/archival_objects/1"
                .to_string(),
            record_type: "tree_node".to_string(),
            pui_parent_id: Some("/repositories/2/archival_objects/1".to_string()),
            record: node_record.clone(),
            pui_auxiliary_json: Some(serde_json::to_string(&node_record).unwrap()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let node_doc = build_index_document(node_input).unwrap();
        assert_eq!(
            node_doc["id"],
            "/repositories/2/resources/10/tree/node_/repositories/2/archival_objects/1"
        );
        assert_eq!(
            node_doc["uri"],
            "/repositories/2/resources/10/tree/node_/repositories/2/archival_objects/1"
        );
        assert_eq!(
            node_doc["pui_parent_id"],
            "/repositories/2/archival_objects/1"
        );
        assert_eq!(node_doc["publish"], "true");
        assert_eq!(node_doc["primary_type"], "tree_node");
        assert_eq!(node_doc["types"], json!(["pui"]));
        let node_meta = node_doc[PUI_AUXILIARY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(node_meta["marker"], PUI_AUXILIARY_METADATA_MARKER);
        assert_eq!(node_meta["auxiliary_type"], "tree_node");

        // 5. tree_node_from_root
        let path_record = json!({
            "42": [{"node": "/repositories/2/resources/10", "offset": 0}]
        });
        let path_input = IndexDocumentInput {
            uri: "/repositories/2/resources/10/tree/node_from_root_42".to_string(),
            record_type: "tree_node_from_root".to_string(),
            pui_parent_id: Some("/repositories/2/archival_objects/42".to_string()),
            record: path_record.clone(),
            pui_auxiliary_json: Some(serde_json::to_string(&path_record).unwrap()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let path_doc = build_index_document(path_input).unwrap();
        assert_eq!(
            path_doc["id"],
            "/repositories/2/resources/10/tree/node_from_root_42"
        );
        assert_eq!(
            path_doc["uri"],
            "/repositories/2/resources/10/tree/node_from_root_42"
        );
        assert_eq!(
            path_doc["pui_parent_id"],
            "/repositories/2/archival_objects/42"
        );
        assert_eq!(path_doc["publish"], "true");
        assert_eq!(path_doc["primary_type"], "tree_node_from_root");
        assert_eq!(path_doc["types"], json!(["pui"]));
        let path_meta = path_doc[PUI_AUXILIARY_DOCUMENTS_KEY].as_object().unwrap();
        assert_eq!(path_meta["marker"], PUI_AUXILIARY_METADATA_MARKER);
        assert_eq!(path_meta["auxiliary_type"], "tree_node_from_root");
    }

    #[test]
    fn test_pui_auxiliary_unicode_and_nested_json() {
        let record = json!({
            "uri": "/repositories/2/archival_objects/Séan_Sánchez_日本語_🎉",
            "title": "Muintir na hÉireann & Sánchez Papers / 記録",
            "nested": {
                "levels": [
                    {"deep": {"key": "luach", "list": ["a", "b", "c"]}}
                ]
            }
        });
        let raw_json = serde_json::to_string(&record).unwrap();
        let unicode_input = IndexDocumentInput {
            uri: "/repositories/2/resources/99/tree/node_/repositories/2/archival_objects/Séan_Sánchez_日本語_🎉".to_string(),
            record_type: "tree_node".to_string(),
            pui_parent_id: Some("/repositories/2/archival_objects/Séan_Sánchez_日本語_🎉".to_string()),
            record: record.clone(),
            pui_auxiliary_json: Some(raw_json.clone()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let doc = build_index_document(unicode_input).unwrap();
        assert_eq!(doc["id"], "/repositories/2/resources/99/tree/node_/repositories/2/archival_objects/Séan_Sánchez_日本語_🎉");
        assert_eq!(
            doc["pui_parent_id"],
            "/repositories/2/archival_objects/Séan_Sánchez_日本語_🎉"
        );
        assert_eq!(doc["json"], raw_json);
        let parsed: Value = serde_json::from_str(doc["json"].as_str().unwrap()).unwrap();
        assert_eq!(
            parsed["title"],
            "Muintir na hÉireann & Sánchez Papers / 記録"
        );
        assert_eq!(parsed["nested"]["levels"][0]["deep"]["key"], "luach");
    }

    #[test]
    fn test_pui_auxiliary_verbatim_json_passthrough() {
        // Noncanonical key ordering in pui_auxiliary_json is preserved verbatim when semantically equal
        let record = json!({"alpha": 1, "beta": 2});
        let noncanonical_json = "{\"beta\":2,\"alpha\":1}".to_string();
        let input = IndexDocumentInput {
            uri: "/repositories/2/resources/10/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/10".to_string()),
            record,
            pui_auxiliary_json: Some(noncanonical_json.clone()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let doc = build_index_document(input).unwrap();
        assert_eq!(doc["json"], noncanonical_json);
    }

    #[test]
    fn test_pui_auxiliary_batch_ordering_and_isolation() {
        let rec1 = json!({"item": 1});
        let rec2 = json!({"item": 2});
        let rec3 = json!([{"item": 3}]);
        let records = vec![
            IndexDocumentInput {
                uri: "/repositories/2/resources/1/ordered_records".to_string(),
                record_type: "resource_ordered_records".to_string(),
                pui_parent_id: Some("/repositories/2/resources/1".to_string()),
                record: rec1.clone(),
                pui_auxiliary_json: Some(serde_json::to_string(&rec1).unwrap()),
                prepare_pui_auxiliary: true,
                ..Default::default()
            },
            IndexDocumentInput {
                uri: "/repositories/2/resources/1/tree/root".to_string(),
                record_type: "tree_root".to_string(),
                pui_parent_id: Some("/repositories/2/resources/1".to_string()),
                record: rec2.clone(),
                pui_auxiliary_json: Some(serde_json::to_string(&rec2).unwrap()),
                prepare_pui_auxiliary: true,
                ..Default::default()
            },
            IndexDocumentInput {
                uri: "/repositories/2/resources/1/tree/waypoint_parent_0".to_string(),
                record_type: "tree_waypoint".to_string(),
                pui_parent_id: Some("/repositories/2/resources/1".to_string()),
                record: rec3.clone(),
                pui_auxiliary_json: Some(serde_json::to_string(&rec3).unwrap()),
                prepare_pui_auxiliary: true,
                ..Default::default()
            },
        ];
        let response = build_index_document_batch(IndexDocumentBatchRequest { records }).unwrap();
        assert_eq!(response.documents.len(), 3);
        assert_eq!(
            response.documents[0]["uri"],
            "/repositories/2/resources/1/ordered_records"
        );
        assert_eq!(
            response.documents[1]["uri"],
            "/repositories/2/resources/1/tree/root"
        );
        assert_eq!(
            response.documents[2]["uri"],
            "/repositories/2/resources/1/tree/waypoint_parent_0"
        );
    }

    #[test]
    fn test_pui_auxiliary_duplicates_supported() {
        let rec = json!({"title": "Root"});
        let input_item = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: rec.clone(),
            pui_auxiliary_json: Some(serde_json::to_string(&rec).unwrap()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        let response = build_index_document_batch(IndexDocumentBatchRequest {
            records: vec![input_item.clone(), input_item],
        })
        .unwrap();
        assert_eq!(response.documents.len(), 2);
        assert_eq!(
            response.documents[0]["id"],
            "/repositories/2/resources/5/tree/root"
        );
        assert_eq!(
            response.documents[1]["id"],
            "/repositories/2/resources/5/tree/root"
        );
    }

    #[test]
    fn test_pui_auxiliary_validation_rejections() {
        let valid_rec = json!({"title": "Root"});
        let valid_json = serde_json::to_string(&valid_rec).unwrap();

        // 1. Missing parent_id
        let no_parent = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: None,
            record: valid_rec.clone(),
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(no_parent).is_err());

        // 2. Empty parent_id
        let empty_parent = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("   ".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(empty_parent).is_err());

        // 3. Null record
        let null_rec = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: Value::Null,
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(null_rec).is_err());

        // 4. Invalid record_type
        let bad_type = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "archival_object".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(bad_type).is_err());

        // 5. tree_waypoint with object instead of array
        let wp_not_array = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/waypoint__0".to_string(),
            record_type: "tree_waypoint".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!({"not": "an_array"}),
            pui_auxiliary_json: Some("{\"not\":\"an_array\"}".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(wp_not_array).is_err());

        // 6. tree_node with array instead of object
        let node_not_obj = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/node_1".to_string(),
            record_type: "tree_node".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!(["not", "an_object"]),
            pui_auxiliary_json: Some("[\"not\",\"an_object\"]".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(node_not_obj).is_err());

        // 7. Cannot combine with enrich
        let comb_enrich = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            enrich: true,
            ..Default::default()
        };
        assert!(build_index_document(comb_enrich).is_err());

        // 8. Cannot combine with prepare_pui_final
        let comb_final = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            prepare_pui_final: true,
            ..Default::default()
        };
        assert!(build_index_document(comb_final).is_err());

        // 9. Cannot combine with prepare_pui_availability
        let comb_avail = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some(valid_json.clone()),
            prepare_pui_auxiliary: true,
            prepare_pui_availability: true,
            ..Default::default()
        };
        assert!(build_index_document(comb_avail).is_err());

        // 10. pui_parent_id without prepare_pui_auxiliary is rejected
        let parent_without_flag = IndexDocumentInput {
            uri: "/repositories/2/resources/5".to_string(),
            record_type: "resource".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!({"title": "Res"}),
            prepare_pui_auxiliary: false,
            ..Default::default()
        };
        assert!(build_index_document(parent_without_flag).is_err());

        // 11. pui_auxiliary_json without prepare_pui_auxiliary is rejected
        let aux_json_without_flag = IndexDocumentInput {
            uri: "/repositories/2/resources/5".to_string(),
            record_type: "resource".to_string(),
            pui_auxiliary_json: Some("{}".to_string()),
            record: json!({"title": "Res"}),
            prepare_pui_auxiliary: false,
            ..Default::default()
        };
        assert!(build_index_document(aux_json_without_flag).is_err());

        // 12. Missing pui_auxiliary_json
        let no_aux_json = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: None,
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(no_aux_json).is_err());

        // 13. Empty pui_auxiliary_json
        let empty_aux_json = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some("   ".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(empty_aux_json).is_err());

        // 14. Malformed JSON in pui_auxiliary_json
        let malformed_aux_json = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some("{not valid json".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(malformed_aux_json).is_err());

        // 15. Semantic mismatch between record and pui_auxiliary_json
        let mismatch_aux_json = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: valid_rec.clone(),
            pui_auxiliary_json: Some("{\"title\":\"Different Title\"}".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(mismatch_aux_json).is_err());

        // 16. Type mismatch: integer vs float
        let int_float_mismatch = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!({"count": 1}),
            pui_auxiliary_json: Some("{\"count\":1.0}".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(int_float_mismatch).is_err());

        // 17. Type mismatch: bool vs int
        let bool_int_mismatch = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!({"flag": true}),
            pui_auxiliary_json: Some("{\"flag\":1}".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(bool_int_mismatch).is_err());

        // 18. Type mismatch: bool vs string
        let bool_str_mismatch = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!({"flag": true}),
            pui_auxiliary_json: Some("{\"flag\":\"true\"}".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(bool_str_mismatch).is_err());

        // 19. Type mismatch: string vs integer
        let str_int_mismatch = IndexDocumentInput {
            uri: "/repositories/2/resources/5/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/5".to_string()),
            record: json!({"val": "42"}),
            pui_auxiliary_json: Some("{\"val\":42}".to_string()),
            prepare_pui_auxiliary: true,
            ..Default::default()
        };
        assert!(build_index_document(str_int_mismatch).is_err());
    }

    #[test]
    fn test_record_inheritance_candidate_default_ao_rules() {
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: false,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {
                            "jsonmodel_type": "note_singlepart",
                            "type": "accessrestrict",
                            "publish": true,
                            "content": ["Inherited access restriction from series"]
                        }
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/archival_objects/99"),
                        level: json!("Serie"),
                        direct: false,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "linked_agents".to_string(),
                    inherit_directly: false,
                    ancestor_index: 1,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {
                            "role": "creator",
                            "ref": "/agents/people/1",
                            "terms": []
                        }
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/1"),
                        level: json!("Fondo"),
                        direct: false,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "language".to_string(),
                    inherit_directly: false,
                    ancestor_index: 1,
                    ancestor_element_indices: None,
                    inherited_value: json!("spa"),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/1"),
                        level: json!("Fondo"),
                        direct: false,
                    },
                },
            ],
            composite_identifiers: Some(CompositeIdentifierPlan {
                pieces: vec![
                    "MS-01".to_string(),
                    "Serie S1".to_string(),
                    "Unidad item-100".to_string(),
                ],
                delimiter: ". ".to_string(),
            }),
        };

        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/100",
            "jsonmodel_type": "archival_object",
            "title": "Item 100 with Inheritance",
            "publish": true,
            "suppressed": false,
            "component_id": "item-100",
            "level": "item",
            "ancestors": [
                {
                    "ref": "/repositories/2/archival_objects/99",
                    "level": "series",
                    "_resolved": {
                        "component_id": "S1",
                        "notes": [
                            {
                                "jsonmodel_type": "note_singlepart",
                                "type": "accessrestrict",
                                "publish": true,
                                "content": ["Inherited access restriction from series"]
                            }
                        ]
                    }
                },
                {
                    "ref": "/repositories/2/resources/1",
                    "level": "collection",
                    "_resolved": {
                        "id_0": "MS-01",
                        "language": "spa",
                        "linked_agents": [
                            {
                                "role": "creator",
                                "ref": "/agents/people/1",
                                "terms": []
                            }
                        ]
                    }
                }
            ],
            "notes": [
                {
                    "jsonmodel_type": "note_singlepart",
                    "type": "scopecontent",
                    "publish": true,
                    "content": ["Direct AO scopecontent"]
                }
            ],
            "linked_agents": []
        });

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/100".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).expect("should build document with inheritance plan");
        let doc_obj = doc.as_object().unwrap();

        let rust_fields = doc_obj
            .get("_archivalist_rust_fields")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(rust_fields.contains(&Value::String(
            PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER.to_string()
        )));

        let cand_proj = doc_obj
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            cand_proj.get("marker").unwrap().as_str().unwrap(),
            PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER
        );
        let cand_record = cand_proj.get("record").unwrap().as_object().unwrap();

        // Candidate retains ancestors
        assert!(cand_record.contains_key("ancestors"));

        // CRITICAL aliasing parity: ancestor 0 notes[0] mutated in place with _inherited
        let anc0 = &cand_record.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc0_notes = anc0
            .get("_resolved")
            .unwrap()
            .get("notes")
            .unwrap()
            .as_array()
            .unwrap();
        let anc0_inh = anc0_notes[0]
            .get("_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(anc0_inh.get("level").unwrap().as_str(), Some("Serie"));
        assert_eq!(anc0_inh.get("direct").unwrap().as_bool(), Some(false));
        assert_eq!(
            anc0_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/archival_objects/99")
        );

        // Ancestor 1 linked_agents[0] mutated in place with _inherited
        let anc1 = &cand_record.get("ancestors").unwrap().as_array().unwrap()[1];
        let anc1_agents = anc1
            .get("_resolved")
            .unwrap()
            .get("linked_agents")
            .unwrap()
            .as_array()
            .unwrap();
        let anc1_inh = anc1_agents[0]
            .get("_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(anc1_inh.get("level").unwrap().as_str(), Some("Fondo"));
        assert_eq!(anc1_inh.get("direct").unwrap().as_bool(), Some(false));
        assert_eq!(
            anc1_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/1")
        );

        // Notes: direct note does NOT have _inherited, inherited note DOES have _inherited
        let notes = cand_record.get("notes").unwrap().as_array().unwrap();
        assert_eq!(notes.len(), 2);
        assert!(!notes[0].as_object().unwrap().contains_key("_inherited"));
        let inherited_note = notes[1].as_object().unwrap();
        let note_inh = inherited_note
            .get("_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            note_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/archival_objects/99")
        );
        assert_eq!(note_inh.get("level").unwrap().as_str(), Some("Serie"));
        assert_eq!(note_inh.get("direct").unwrap().as_bool(), Some(false));

        // Linked agents: creator agent has _inherited
        let agents = cand_record
            .get("linked_agents")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(agents.len(), 1);
        let agent_inh = agents[0]
            .as_object()
            .unwrap()
            .get("_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            agent_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/1")
        );
        assert_eq!(agent_inh.get("level").unwrap().as_str(), Some("Fondo"));

        // Language: scalar has language_inherited
        assert_eq!(cand_record.get("language").unwrap().as_str(), Some("spa"));
        let lang_inh = cand_record
            .get("language_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            lang_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/1")
        );
        assert_eq!(lang_inh.get("level").unwrap().as_str(), Some("Fondo"));

        // Composite identifier
        assert_eq!(
            cand_record.get("_composite_identifier").unwrap().as_str(),
            Some("MS-01. Serie S1. Unidad item-100")
        );

        // Final PUI document json stripped ancestors
        let final_proj = doc_obj
            .get(PUI_FINAL_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap();
        let final_json_str = final_proj.get("json").unwrap().as_str().unwrap();
        let final_json: Value = serde_json::from_str(final_json_str).unwrap();
        assert!(!final_json.as_object().unwrap().contains_key("ancestors"));
    }

    #[test]
    fn test_record_inheritance_candidate_repeated_notes_rules() {
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                // Rule 1: accessrestrict note (child lacks it -> inherits from ancestor 0)
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {
                            "jsonmodel_type": "note_singlepart",
                            "type": "accessrestrict",
                            "publish": true,
                            "content": ["Series accessrestrict"]
                        }
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/archival_objects/99"),
                        level: json!("series"),
                        direct: true,
                    },
                },
                // Rule 2: langmaterial note (child lacks it -> inherits from ancestor 1)
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: false,
                    ancestor_index: 1,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {
                            "jsonmodel_type": "note_singlepart",
                            "type": "langmaterial",
                            "publish": true,
                            "content": ["Collection langmaterial"]
                        }
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/1"),
                        level: json!("collection"),
                        direct: false,
                    },
                },
            ],
            composite_identifiers: None,
        };

        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/105",
            "jsonmodel_type": "archival_object",
            "title": "Item 105 Repeated Notes",
            "publish": true,
            "suppressed": false,
            "ancestors": [
                {
                    "ref": "/repositories/2/archival_objects/99",
                    "level": "series",
                    "_resolved": {
                        "notes": [
                            {
                                "jsonmodel_type": "note_singlepart",
                                "type": "accessrestrict",
                                "publish": true,
                                "content": ["Series accessrestrict"]
                            }
                        ]
                    }
                },
                {
                    "ref": "/repositories/2/resources/1",
                    "level": "collection",
                    "_resolved": {
                        "notes": [
                            {
                                "jsonmodel_type": "note_singlepart",
                                "type": "langmaterial",
                                "publish": true,
                                "content": ["Collection langmaterial"]
                            }
                        ]
                    }
                }
            ],
            "notes": [
                {
                    "jsonmodel_type": "note_singlepart",
                    "type": "scopecontent",
                    "publish": true,
                    "content": ["Direct child scopecontent"]
                }
            ]
        });

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/105".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // Check ancestor mutations
        let anc0 = &cand_record.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc0_notes = anc0
            .get("_resolved")
            .unwrap()
            .get("notes")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            anc0_notes[0]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(true)
        );

        let anc1 = &cand_record.get("ancestors").unwrap().as_array().unwrap()[1];
        let anc1_notes = anc1
            .get("_resolved")
            .unwrap()
            .get("notes")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            anc1_notes[0]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(false)
        );

        let notes = cand_record.get("notes").unwrap().as_array().unwrap();
        assert_eq!(notes.len(), 3);

        // 1. Direct scopecontent note (no _inherited)
        assert_eq!(notes[0].get("type").unwrap().as_str(), Some("scopecontent"));
        assert!(!notes[0].as_object().unwrap().contains_key("_inherited"));

        // 2. Inherited accessrestrict note from series (direct: true)
        assert_eq!(
            notes[1].get("type").unwrap().as_str(),
            Some("accessrestrict")
        );
        let inh1 = notes[1].get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            inh1.get("ref").unwrap().as_str(),
            Some("/repositories/2/archival_objects/99")
        );
        assert_eq!(inh1.get("direct").unwrap().as_bool(), Some(true));

        // 3. Inherited langmaterial note from collection (direct: false)
        assert_eq!(notes[2].get("type").unwrap().as_str(), Some("langmaterial"));
        let inh2 = notes[2].get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            inh2.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/1")
        );
        assert_eq!(inh2.get("direct").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn test_record_inheritance_candidate_direct_scalar_array_flattening() {
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/101",
            "jsonmodel_type": "archival_object",
            "title": "Item 101 Direct & Flattening",
            "publish": true,
            "suppressed": false,
            "flag_scalar": false,
            "ancestors": [
                {
                    "ref": "/repositories/2/archival_objects/98",
                    "level": "subseries",
                    "_resolved": {
                        "nested_notes": [
                            [
                                {"type": "note1", "content": "N1"},
                                {"type": "note2", "content": "N2"}
                            ],
                            {"type": "note3", "content": "N3"}
                        ],
                        "tags": ["tagA", "tagB"],
                        "flag_scalar": "active"
                    }
                }
            ],
            "nested_notes": []
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                RecordInheritanceRulePlan {
                    property: "nested_notes".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0, 1]),
                    inherited_value: json!([
                        [
                            {"type": "note1", "content": "N1"},
                            {"type": "note2", "content": "N2"}
                        ],
                        {"type": "note3", "content": "N3"}
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/archival_objects/98"),
                        level: json!("subseries"),
                        direct: true,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "tags".to_string(),
                    inherit_directly: false,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0, 1]),
                    inherited_value: json!(["tagA", "tagB"]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/archival_objects/98"),
                        level: json!("subseries"),
                        direct: false,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "flag_scalar".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: None,
                    inherited_value: json!("active"),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/archival_objects/98"),
                        level: json!("subseries"),
                        direct: true,
                    },
                },
            ],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/101".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // nested_notes should be recursively flattened from [[N1, N2], N3] to [N1, N2, N3]
        let notes = cand_record.get("nested_notes").unwrap().as_array().unwrap();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].get("type").unwrap().as_str(), Some("note1"));
        assert_eq!(notes[1].get("type").unwrap().as_str(), Some("note2"));
        assert_eq!(notes[2].get("type").unwrap().as_str(), Some("note3"));
        // inherit_directly was true -> note3 had _inherited with direct: true
        assert_eq!(
            notes[2]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        // The nested array in ancestor_val triggers nested_notes_inherited on record
        assert!(cand_record.contains_key("nested_notes_inherited"));

        // tags: scalar strings in array -> tags_inherited on record
        let tags = cand_record.get("tags").unwrap().as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].as_str(), Some("tagA"));
        assert_eq!(tags[1].as_str(), Some("tagB"));
        assert!(cand_record.contains_key("tags_inherited"));

        // flag_scalar: was false on record -> inherited "active" with direct: true
        assert_eq!(
            cand_record.get("flag_scalar").unwrap().as_str(),
            Some("active")
        );
        let flag_inh = cand_record
            .get("flag_scalar_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(flag_inh.get("direct").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_record_inheritance_candidate_first_qualifying_ancestor() {
        // Ancestor 1 has no qualifying note; Ancestor 2 has qualifying note
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/102",
            "jsonmodel_type": "archival_object",
            "title": "Item 102 First Qualifying",
            "publish": true,
            "suppressed": false,
            "ancestors": [
                {
                    "ref": "/repositories/2/archival_objects/90",
                    "level": "series",
                    "_resolved": {}
                },
                {
                    "ref": "/repositories/2/resources/2",
                    "level": "collection",
                    "_resolved": {
                        "language": "fre"
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![RecordInheritanceRulePlan {
                property: "language".to_string(),
                inherit_directly: false,
                ancestor_index: 1,
                ancestor_element_indices: None,
                inherited_value: json!("fre"),
                inherited_props: InheritanceProps {
                    ref_val: json!("/repositories/2/resources/2"),
                    level: json!("collection"),
                    direct: false,
                },
            }],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/102".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        assert_eq!(cand_record.get("language").unwrap().as_str(), Some("fre"));
        let lang_inh = cand_record
            .get("language_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            lang_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/2")
        );
    }

    #[test]
    fn test_record_inheritance_candidate_composite_identifier_variations() {
        // 1. Delimiter "/" with include_level: true -> "/ "
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/103",
            "jsonmodel_type": "archival_object",
            "title": "Item 103 Composite Id",
            "publish": true,
            "suppressed": false,
            "component_id": "c103",
            "level": "item",
            "ancestors": [
                {
                    "ref": "/repositories/2/archival_objects/91",
                    "level": "series",
                    "_resolved": {
                        "component_id": "s91"
                    }
                },
                {
                    "ref": "/repositories/2/resources/3",
                    "level": "collection",
                    "_resolved": {
                        "id_0": "COLL",
                        "id_1": "2026",
                        "id_2": "B"
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![],
            composite_identifiers: Some(CompositeIdentifierPlan {
                pieces: vec![
                    "COLL/2026/B".to_string(),
                    "series s91".to_string(),
                    "item c103".to_string(),
                ],
                delimiter: "/ ".to_string(),
            }),
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/103".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        assert_eq!(
            cand_record.get("_composite_identifier").unwrap().as_str(),
            Some("COLL/2026/B/ series s91/ item c103")
        );

        // 2. Delimiter " " with include_level: true -> " "
        let plan_space = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![],
            composite_identifiers: Some(CompositeIdentifierPlan {
                pieces: vec!["collection r3".to_string(), "item c103".to_string()],
                delimiter: " ".to_string(),
            }),
        };
        let input_space = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/103".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/103",
                "jsonmodel_type": "archival_object",
                "publish": true,
                "component_id": "c103",
                "level": "item"
            }),
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan_space),
            ..Default::default()
        };
        let doc_space = build_index_document(input_space).unwrap();
        let cand_space = doc_space
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap();
        assert_eq!(
            cand_space.get("_composite_identifier").unwrap().as_str(),
            Some("collection r3 item c103")
        );

        // 3. Exclude own component_id when component_id_inherited: true
        let plan_exclude = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![],
            composite_identifiers: Some(CompositeIdentifierPlan {
                pieces: vec!["r3".to_string()],
                delimiter: ".".to_string(),
            }),
        };
        let input_exclude = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/103".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/103",
                "jsonmodel_type": "archival_object",
                "publish": true,
                "component_id": "c103"
            }),
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan_exclude),
            ..Default::default()
        };
        let doc_exclude = build_index_document(input_exclude).unwrap();
        let cand_exclude = doc_exclude
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap();
        assert_eq!(
            cand_exclude.get("_composite_identifier").unwrap().as_str(),
            Some("r3")
        );
    }

    #[test]
    fn test_record_inheritance_candidate_unicode_duplicates_and_numbers() {
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/104",
            "jsonmodel_type": "archival_object",
            "title": "Unicodé — 📁 Papeles de Ramón Gómez 🚀",
            "publish": true,
            "suppressed": false,
            "int_val": 42,
            "float_val": 12.345,
            "bool_val": true,
            "tags": ["dupe", "dupe"],
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/4",
                    "level": "collection",
                    "_resolved": {
                        "tags": ["dupe", "extra_utf8_🔥"]
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![RecordInheritanceRulePlan {
                property: "tags".to_string(),
                inherit_directly: false,
                ancestor_index: 0,
                ancestor_element_indices: Some(vec![0, 1]),
                inherited_value: json!(["dupe", "extra_utf8_🔥"]),
                inherited_props: InheritanceProps {
                    ref_val: json!("/repositories/2/resources/4"),
                    level: json!("collection"),
                    direct: false,
                },
            }],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/104".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // Check Unicode title
        assert_eq!(
            cand_record.get("title").unwrap().as_str(),
            Some("Unicodé — 📁 Papeles de Ramón Gómez 🚀")
        );

        // Check integer remains integer
        let int_val = cand_record.get("int_val").unwrap().as_i64();
        assert_eq!(int_val, Some(42));

        // Check float remains float
        let float_val = cand_record.get("float_val").unwrap().as_f64();
        assert_eq!(float_val, Some(12.345));

        // Check bool remains bool
        assert_eq!(cand_record.get("bool_val").unwrap().as_bool(), Some(true));

        // Tags had 2 dupes, inherited 2 more -> total 4 items, duplicates preserved
        let tags = cand_record.get("tags").unwrap().as_array().unwrap();
        assert_eq!(tags.len(), 4);
        assert_eq!(tags[0].as_str(), Some("dupe"));
        assert_eq!(tags[1].as_str(), Some("dupe"));
        assert_eq!(tags[2].as_str(), Some("dupe"));
        assert_eq!(tags[3].as_str(), Some("extra_utf8_🔥"));
    }

    #[test]
    fn test_record_inheritance_candidate_batch_isolation() {
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![],
            composite_identifiers: None,
        };

        let req = IndexDocumentBatchRequest {
            records: vec![
                IndexDocumentInput {
                    uri: "/repositories/2/resources/1".to_string(),
                    record_type: "resource".to_string(),
                    repository: json!(2),
                    record: json!({
                        "uri": "/repositories/2/resources/1",
                        "jsonmodel_type": "resource",
                        "publish": true
                    }),
                    prepare_pui_final: true,
                    pui_candidate_record: Some(json!({
                        "uri": "/repositories/2/resources/1",
                        "jsonmodel_type": "resource",
                        "publish": true
                    })),
                    ..Default::default()
                },
                IndexDocumentInput {
                    uri: "/repositories/2/archival_objects/2".to_string(),
                    record_type: "archival_object".to_string(),
                    repository: json!(2),
                    record: json!({
                        "uri": "/repositories/2/archival_objects/2",
                        "jsonmodel_type": "archival_object",
                        "publish": true
                    }),
                    prepare_pui_final: true,
                    record_inheritance_plan: Some(plan),
                    ..Default::default()
                },
                IndexDocumentInput {
                    uri: "/repositories/2/resources/3".to_string(),
                    record_type: "resource".to_string(),
                    repository: json!(2),
                    record: json!({
                        "uri": "/repositories/2/resources/3",
                        "jsonmodel_type": "resource",
                        "publish": true
                    }),
                    enrich: true,
                    ..Default::default()
                },
            ],
        };

        let resp = build_index_document_batch(req).unwrap();
        assert_eq!(resp.documents.len(), 3);

        // Record 1: has PUI_FINAL and PUI_NOTES but NO candidate marker
        let doc1 = resp.documents[0].as_object().unwrap();
        let fields1 = doc1
            .get("_archivalist_rust_fields")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(!fields1.contains(&Value::String(
            PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER.to_string()
        )));
        assert!(!doc1.contains_key(PUI_CANDIDATE_DOCUMENTS_KEY));

        // Record 2: has candidate marker and candidate document
        let doc2 = resp.documents[1].as_object().unwrap();
        let fields2 = doc2
            .get("_archivalist_rust_fields")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(fields2.contains(&Value::String(
            PUI_RECORD_INHERITANCE_CANDIDATE_METADATA_MARKER.to_string()
        )));
        assert!(doc2.contains_key(PUI_CANDIDATE_DOCUMENTS_KEY));

        // Record 3: staff enrich
        let doc3 = resp.documents[2].as_object().unwrap();
        assert!(!doc3.contains_key(PUI_CANDIDATE_DOCUMENTS_KEY));
        assert!(!doc3.contains_key(PUI_FINAL_DOCUMENTS_KEY));
    }

    #[test]
    fn test_record_inheritance_candidate_validation_rejections() {
        let valid_plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![],
            composite_identifiers: None,
        };

        // 1. Both plan and candidate provided
        let err1 = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/1".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/1",
                "jsonmodel_type": "archival_object",
                "publish": true
            }),
            prepare_pui_final: true,
            pui_candidate_record: Some(json!({
                "uri": "/repositories/2/archival_objects/1",
                "jsonmodel_type": "archival_object"
            })),
            record_inheritance_plan: Some(valid_plan.clone()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err1
            .to_string()
            .contains("cannot provide both record_inheritance_plan and pui_candidate_record"));

        // 2. Plan provided with prepare_pui_final: false
        let err2 = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/archival_objects/1".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/1",
                "jsonmodel_type": "archival_object",
                "publish": true
            }),
            prepare_pui_final: false,
            record_inheritance_plan: Some(valid_plan.clone()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err2
            .to_string()
            .contains("record_inheritance_plan is only allowed when prepare_pui_final is true"));

        // 3. Plan provided with prepare_pui_auxiliary: true
        let err3 = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/resources/1/tree/root".to_string(),
            record_type: "tree_root".to_string(),
            pui_parent_id: Some("/repositories/2/resources/1".to_string()),
            record: json!({}),
            pui_auxiliary_json: Some("{}".to_string()),
            prepare_pui_auxiliary: true,
            record_inheritance_plan: Some(valid_plan.clone()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err3.to_string().contains("cannot set staff enrichment or pui final/availability fields when prepare_pui_auxiliary is true"));

        // 4. Plan record_type mismatch
        let err4 = build_index_document(IndexDocumentInput {
            uri: "/repositories/2/resources/1".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/resources/1",
                "jsonmodel_type": "resource",
                "publish": true
            }),
            prepare_pui_final: true,
            record_inheritance_plan: Some(valid_plan),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err4
            .to_string()
            .contains("record_inheritance_plan record_type 'archival_object' does not match wrapper record_type 'resource'"));
    }

    #[test]
    fn test_record_inheritance_candidate_duplicate_refs_with_different_levels_and_values() {
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {
                            "jsonmodel_type": "note_singlepart",
                            "type": "scopecontent",
                            "content": ["First note from dupe ref"]
                        }
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/dupe"),
                        level: json!("collection_level_A"),
                        direct: true,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: false,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![1]),
                    inherited_value: json!([
                        {
                            "jsonmodel_type": "note_singlepart",
                            "type": "accessrestrict",
                            "content": ["Second note from same dupe ref"]
                        }
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/dupe"),
                        level: json!("series_level_B"),
                        direct: false,
                    },
                },
            ],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/200".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/200",
                "jsonmodel_type": "archival_object",
                "publish": true,
                "notes": [],
                "ancestors": [
                    {
                        "ref": "/repositories/2/resources/dupe",
                        "level": "collection_level_A",
                        "_resolved": {
                            "notes": [
                                {
                                    "jsonmodel_type": "note_singlepart",
                                    "type": "scopecontent",
                                    "content": ["First note from dupe ref"]
                                },
                                {
                                    "jsonmodel_type": "note_singlepart",
                                    "type": "accessrestrict",
                                    "content": ["Second note from same dupe ref"]
                                }
                            ]
                        }
                    }
                ]
            }),
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        let notes = cand_record.get("notes").unwrap().as_array().unwrap();
        assert_eq!(notes.len(), 2);

        // First note: level collection_level_A, direct: true
        assert_eq!(notes[0].get("type").unwrap().as_str(), Some("scopecontent"));
        let inh0 = notes[0].get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            inh0.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/dupe")
        );
        assert_eq!(
            inh0.get("level").unwrap().as_str(),
            Some("collection_level_A")
        );
        assert_eq!(inh0.get("direct").unwrap().as_bool(), Some(true));

        // Second note: level series_level_B, direct: false (NOT overwritten by collection_level_A!)
        assert_eq!(
            notes[1].get("type").unwrap().as_str(),
            Some("accessrestrict")
        );
        let inh1 = notes[1].get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            inh1.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/dupe")
        );
        assert_eq!(inh1.get("level").unwrap().as_str(), Some("series_level_B"));
        assert_eq!(inh1.get("direct").unwrap().as_bool(), Some(false));

        // Selected ancestor in-place mutations
        let anc0 = &cand_record.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc0_notes = anc0
            .get("_resolved")
            .unwrap()
            .get("notes")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            anc0_notes[0]
                .get("_inherited")
                .unwrap()
                .get("level")
                .unwrap()
                .as_str(),
            Some("collection_level_A")
        );
        assert_eq!(
            anc0_notes[1]
                .get("_inherited")
                .unwrap()
                .get("level")
                .unwrap()
                .as_str(),
            Some("series_level_B")
        );
    }

    #[test]
    fn test_record_inheritance_candidate_null_and_missing_and_non_string_ref() {
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                // 1. Null ref, null level
                RecordInheritanceRulePlan {
                    property: "language".to_string(),
                    inherit_directly: false,
                    ancestor_index: 0,
                    ancestor_element_indices: None,
                    inherited_value: json!("lat"),
                    inherited_props: InheritanceProps {
                        ref_val: Value::Null,
                        level: Value::Null,
                        direct: false,
                    },
                },
                // 2. Integer ref (non-string JSON ref)
                RecordInheritanceRulePlan {
                    property: "edition".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: None,
                    inherited_value: json!("1st ed"),
                    inherited_props: InheritanceProps {
                        ref_val: json!(12345),
                        level: json!("item"),
                        direct: true,
                    },
                },
            ],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/201".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: json!({
                "uri": "/repositories/2/archival_objects/201",
                "jsonmodel_type": "archival_object",
                "publish": true,
                "ancestors": [
                    {
                        "ref": null,
                        "level": null,
                        "_resolved": {
                            "language": "lat",
                            "edition": "1st ed"
                        }
                    }
                ]
            }),
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand_record = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // Language: ref is null, level is null
        assert_eq!(cand_record.get("language").unwrap().as_str(), Some("lat"));
        let lang_inh = cand_record
            .get("language_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert!(lang_inh.get("ref").unwrap().is_null());
        assert!(lang_inh.get("level").unwrap().is_null());
        assert_eq!(lang_inh.get("direct").unwrap().as_bool(), Some(false));

        // Edition: ref is integer 12345
        assert_eq!(cand_record.get("edition").unwrap().as_str(), Some("1st ed"));
        let ed_inh = cand_record
            .get("edition_inherited")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(ed_inh.get("ref").unwrap().as_i64(), Some(12345));
        assert_eq!(ed_inh.get("level").unwrap().as_str(), Some("item"));
        assert_eq!(ed_inh.get("direct").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_record_inheritance_candidate_serde_exact_keys_and_types() {
        // 1. Extra field in RecordInheritancePlan
        let extra_plan_field = json!({
            "record_type": "archival_object",
            "rules": [],
            "unknown_field": 123
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(extra_plan_field).is_err());

        // 2. Extra field in RecordInheritanceRulePlan
        let extra_rule_field = json!({
            "record_type": "archival_object",
            "rules": [
                {
                    "property": "notes",
                    "inherit_directly": true,
                    "ancestor_index": 0,
                    "inherited_value": [],
                    "inherited_props": {
                        "ref": "/res/1",
                        "level": "collection",
                        "direct": true
                    },
                    "extra_key": "not_allowed"
                }
            ]
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(extra_rule_field).is_err());

        // 3. Extra field in InheritanceProps
        let extra_props_field = json!({
            "record_type": "archival_object",
            "rules": [
                {
                    "property": "notes",
                    "inherit_directly": true,
                    "ancestor_index": 0,
                    "inherited_value": [],
                    "inherited_props": {
                        "ref": "/res/1",
                        "level": "collection",
                        "direct": true,
                        "extra_props": true
                    }
                }
            ]
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(extra_props_field).is_err());

        // 4. Extra field in CompositeIdentifierPlan
        let extra_composite_field = json!({
            "record_type": "archival_object",
            "rules": [],
            "composite_identifiers": {
                "pieces": ["a", "b"],
                "delimiter": ".",
                "extra": 999
            }
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(extra_composite_field).is_err());

        // 5. Missing required boolean field inherit_directly
        let missing_inherit_directly = json!({
            "record_type": "archival_object",
            "rules": [
                {
                    "property": "notes",
                    "ancestor_index": 0,
                    "inherited_value": [],
                    "inherited_props": {
                        "ref": "/res/1",
                        "level": "collection",
                        "direct": true
                    }
                }
            ]
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(missing_inherit_directly).is_err());

        // 6. Missing required boolean field direct in inherited_props
        let missing_direct = json!({
            "record_type": "archival_object",
            "rules": [
                {
                    "property": "notes",
                    "inherit_directly": true,
                    "ancestor_index": 0,
                    "inherited_value": [],
                    "inherited_props": {
                        "ref": "/res/1",
                        "level": "collection"
                    }
                }
            ]
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(missing_direct).is_err());

        // 7. Type mismatch: inherit_directly as string instead of bool
        let type_mismatch_rule = json!({
            "record_type": "archival_object",
            "rules": [
                {
                    "property": "notes",
                    "inherit_directly": "true",
                    "ancestor_index": 0,
                    "inherited_value": [],
                    "inherited_props": {
                        "ref": "/res/1",
                        "level": "collection",
                        "direct": true
                    }
                }
            ]
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(type_mismatch_rule).is_err());

        // 8. Type mismatch: pieces as string instead of array of strings
        let type_mismatch_composite = json!({
            "record_type": "archival_object",
            "rules": [],
            "composite_identifiers": {
                "pieces": "not_an_array",
                "delimiter": "."
            }
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(type_mismatch_composite).is_err());

        // 9. Missing required field ancestor_index in RecordInheritanceRulePlan
        let missing_ancestor_index = json!({
            "record_type": "archival_object",
            "rules": [
                {
                    "property": "notes",
                    "inherit_directly": true,
                    "inherited_value": [],
                    "inherited_props": {
                        "ref": "/res/1",
                        "level": "collection",
                        "direct": true
                    }
                }
            ]
        });
        assert!(serde_json::from_value::<RecordInheritancePlan>(missing_ancestor_index).is_err());
    }

    #[test]
    fn test_record_inheritance_candidate_scalar_hash_values() {
        // Scalar Hash property (not array) mutated in both destination and ancestor
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/301",
            "jsonmodel_type": "archival_object",
            "publish": true,
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/10",
                    "level": "collection",
                    "_resolved": {
                        "custom_metadata": {
                            "scope_summary": "High-level summary",
                            "code": 42
                        }
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![RecordInheritanceRulePlan {
                property: "custom_metadata".to_string(),
                inherit_directly: true,
                ancestor_index: 0,
                ancestor_element_indices: None,
                inherited_value: json!({
                    "scope_summary": "High-level summary",
                    "code": 42
                }),
                inherited_props: InheritanceProps {
                    ref_val: json!("/repositories/2/resources/10"),
                    level: json!("collection"),
                    direct: true,
                },
            }],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/301".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // Destination has custom_metadata with _inherited inside the Hash, and custom_metadata_inherited on root
        let meta = cand.get("custom_metadata").unwrap().as_object().unwrap();
        assert_eq!(meta.get("code").unwrap().as_i64(), Some(42));
        let dest_inh = meta.get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            dest_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/10")
        );
        assert_eq!(dest_inh.get("direct").unwrap().as_bool(), Some(true));

        assert!(!cand.contains_key("custom_metadata_inherited"));

        // Selected ancestor in cand also has _inherited inside its custom_metadata hash!
        let anc0 = &cand.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc_meta = anc0
            .get("_resolved")
            .unwrap()
            .get("custom_metadata")
            .unwrap()
            .as_object()
            .unwrap();
        let anc_inh = anc_meta.get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            anc_inh.get("ref").unwrap().as_str(),
            Some("/repositories/2/resources/10")
        );
        assert_eq!(anc_inh.get("direct").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn test_record_inheritance_candidate_mixed_array_values() {
        // Array property containing both Hash objects and scalar values
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/302",
            "jsonmodel_type": "archival_object",
            "publish": true,
            "mixed_items": [],
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/11",
                    "level": "collection",
                    "_resolved": {
                        "mixed_items": [
                            {"type": "item_obj", "val": 100},
                            "scalar_entry_string"
                        ]
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![RecordInheritanceRulePlan {
                property: "mixed_items".to_string(),
                inherit_directly: true,
                ancestor_index: 0,
                ancestor_element_indices: Some(vec![0, 1]),
                inherited_value: json!([
                    {"type": "item_obj", "val": 100},
                    "scalar_entry_string"
                ]),
                inherited_props: InheritanceProps {
                    ref_val: json!("/repositories/2/resources/11"),
                    level: json!("collection"),
                    direct: true,
                },
            }],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/302".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        let items = cand.get("mixed_items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 2);

        // Object item has _inherited
        let item0 = items[0].as_object().unwrap();
        assert_eq!(item0.get("val").unwrap().as_i64(), Some(100));
        assert!(item0.contains_key("_inherited"));

        // Scalar item does not have _inherited, but presence of scalar triggers mixed_items_inherited on root
        assert_eq!(items[1].as_str(), Some("scalar_entry_string"));
        assert!(cand.contains_key("mixed_items_inherited"));

        // Ancestor 0 object item also mutated in-place with _inherited
        let anc0 = &cand.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc_items = anc0
            .get("_resolved")
            .unwrap()
            .get("mixed_items")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(anc_items[0].as_object().unwrap().contains_key("_inherited"));
        assert_eq!(anc_items[1].as_str(), Some("scalar_entry_string"));
    }

    #[test]
    fn test_record_inheritance_candidate_repeated_rules_same_ancestor_and_property() {
        // Two rules targeting the same ancestor index and same property
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/303",
            "jsonmodel_type": "archival_object",
            "publish": true,
            "notes": [],
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/12",
                    "level": "collection",
                    "_resolved": {
                        "notes": [
                            {"type": "rule1_note", "content": ["Content 1"]},
                            {"type": "rule2_note", "content": ["Content 2"]}
                        ]
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {"type": "rule1_note", "content": ["Content 1"]}
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/12"),
                        level: json!("collection"),
                        direct: true,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: false,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![1]),
                    inherited_value: json!([
                        {"type": "rule2_note", "content": ["Content 2"]}
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/12"),
                        level: json!("collection"),
                        direct: false,
                    },
                },
            ],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/303".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        let notes = cand.get("notes").unwrap().as_array().unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(
            notes[0]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            notes[1]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(false)
        );

        let anc0 = &cand.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc_notes = anc0
            .get("_resolved")
            .unwrap()
            .get("notes")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            anc_notes[0]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            anc_notes[1]
                .get("_inherited")
                .unwrap()
                .get("direct")
                .unwrap()
                .as_bool(),
            Some(false)
        );
    }

    #[test]
    fn test_record_inheritance_candidate_progressive_rules_see_prior_inherited_metadata() {
        // First rule inherits linked_agents and mutates ancestor 0.
        // Second rule inherits notes from ancestor 0. Ancestor 0 retains linked_agents mutation.
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/304",
            "jsonmodel_type": "archival_object",
            "publish": true,
            "linked_agents": [],
            "notes": [],
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/13",
                    "level": "collection",
                    "_resolved": {
                        "linked_agents": [
                            {"ref": "/agents/people/1", "role": "creator"}
                        ],
                        "notes": [
                            {"type": "abstract", "content": ["Abstract text"]}
                        ]
                    }
                }
            ]
        });

        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                RecordInheritanceRulePlan {
                    property: "linked_agents".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {"ref": "/agents/people/1", "role": "creator"}
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/13"),
                        level: json!("collection"),
                        direct: true,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "notes".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {"type": "abstract", "content": ["Abstract text"]}
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: json!("/repositories/2/resources/13"),
                        level: json!("collection"),
                        direct: true,
                    },
                },
            ],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/304".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // Both properties inherited in cand
        assert_eq!(
            cand.get("linked_agents").unwrap().as_array().unwrap().len(),
            1
        );
        assert_eq!(cand.get("notes").unwrap().as_array().unwrap().len(), 1);

        // Ancestor has both properties mutated with _inherited
        let anc0 = &cand.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc_res = anc0.get("_resolved").unwrap().as_object().unwrap();
        let anc_agents = anc_res.get("linked_agents").unwrap().as_array().unwrap();
        let anc_notes = anc_res.get("notes").unwrap().as_array().unwrap();
        assert!(anc_agents[0]
            .as_object()
            .unwrap()
            .contains_key("_inherited"));
        assert!(anc_notes[0].as_object().unwrap().contains_key("_inherited"));
    }

    #[test]
    fn test_record_inheritance_candidate_custom_inherit_if_deep_copied_hashes() {
        let ao_record = json!({
            "uri": "/repositories/2/archival_objects/999",
            "jsonmodel_type": "archival_object",
            "publish": true,
            "notes": [],
            "ancestors": [
                {
                    "ref": "/repositories/2/resources/55",
                    "level": "collection",
                    "_resolved": {
                        "notes": [
                            {
                                "jsonmodel_type": "note_singlepart",
                                "type": "scopecontent",
                                "publish": true,
                                "content": ["Scope content from ancestor"]
                            }
                        ]
                    }
                }
            ]
        });

        // Exact plan shape from custom inherit_if returning deep-copied equal Hashes:
        // ancestor_element_indices is Some(vec![]) because identity matching did not find the same object references
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![RecordInheritanceRulePlan {
                property: "notes".to_string(),
                inherit_directly: true,
                ancestor_index: 0,
                ancestor_element_indices: Some(vec![]),
                inherited_value: json!([
                    {
                        "jsonmodel_type": "note_singlepart",
                        "type": "scopecontent",
                        "publish": true,
                        "content": ["Scope content from ancestor"]
                    }
                ]),
                inherited_props: InheritanceProps {
                    ref_val: json!("/repositories/2/resources/55"),
                    level: json!("collection_translated_1"),
                    direct: true,
                },
            }],
            composite_identifiers: None,
        };

        let input = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/999".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: ao_record,
            prepare_pui_final: true,
            record_inheritance_plan: Some(plan),
            ..Default::default()
        };

        let doc = build_index_document(input).unwrap();
        let cand = doc
            .get(PUI_CANDIDATE_DOCUMENTS_KEY)
            .unwrap()
            .get("record")
            .unwrap()
            .as_object()
            .unwrap();

        // Destination copies MUST have _inherited with exact translated level and ref
        let notes = cand.get("notes").unwrap().as_array().unwrap();
        assert_eq!(notes.len(), 1);
        let dest_note = notes[0].as_object().unwrap();
        assert!(dest_note.contains_key("_inherited"));
        let dest_inherited = dest_note.get("_inherited").unwrap().as_object().unwrap();
        assert_eq!(
            dest_inherited.get("ref").unwrap(),
            "/repositories/2/resources/55"
        );
        assert_eq!(
            dest_inherited.get("level").unwrap(),
            "collection_translated_1"
        );
        assert_eq!(
            dest_inherited.get("direct").unwrap(),
            &serde_json::Value::Bool(true)
        );

        // Ancestor _resolved MUST NOT have _inherited!
        let anc0 = &cand.get("ancestors").unwrap().as_array().unwrap()[0];
        let anc_res = anc0.get("_resolved").unwrap().as_object().unwrap();
        let anc_notes = anc_res.get("notes").unwrap().as_array().unwrap();
        assert_eq!(anc_notes.len(), 1);
        let raw_note = anc_notes[0].as_object().unwrap();
        assert!(!raw_note.contains_key("_inherited"));
    }

    #[test]
    fn test_pui_common_metadata_seven_groups() {
        let record = json!({
            "uri": "/repositories/2/resources/100",
            "jsonmodel_type": "resource",
            "title": "Resource 100",
            "publish": true,
            "created_by": "archivist",
            "last_modified_by": "lead",
            "user_mtime": "2026-10-19T12:00:00Z",
            "system_mtime": "2026-10-19T12:00:01Z",
            "create_time": "2026-10-01T00:00:00Z",
            "level": "otherlevel",
            "other_level": "subfonds",
            "subjects": [
                {
                    "ref": "/subjects/1",
                    "_resolved": {"title": "Local History"}
                },
                {
                    "ref": "/subjects/2",
                    "_resolved": {"title": null} // should be compacted out
                },
                {
                    "ref": "/subjects/3",
                    "_resolved": {"title": "Archives"}
                }
            ],
            "linked_agents": [
                {
                    "ref": "/agents/people/1",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Doe, Jane"},
                        "publish": true
                    }
                },
                {
                    "ref": "/agents/corporate_entities/2",
                    "role": "subject",
                    "_resolved": {
                        "display_name": {"sort_name": "Example Historical Society"},
                        "publish": false
                    }
                }
            ],
            "dates": [
                {
                    "date_type": "inclusive",
                    "begin": "1920-01-01",
                    "end": "1925-12-31"
                },
                {
                    "date_type": "single",
                    "begin": "1950-05-01"
                }
            ],
            "notes": [
                {
                    "type": "abstract",
                    "content": ["First paragraph", "Second paragraph"]
                },
                {
                    "type": "scopecontent",
                    "subnotes": [{"content": "Scope text"}]
                }
            ],
            "extents": [
                {
                    "portion": "part",
                    "number": "5",
                    "extent_type": "boxes"
                },
                {
                    "portion": "whole",
                    "number": "1",
                    "extent_type": "linear_metres"
                }
            ]
        });

        let inp = IndexDocumentInput {
            uri: "/repositories/2/resources/100".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };

        let doc = build_index_document(inp).expect("should build common metadata document");
        let doc_obj = doc.as_object().unwrap();

        // Check markers
        let rust_fields = doc_obj
            .get("_archivalist_rust_fields")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(rust_fields, &vec![json!(PUI_COMMON_METADATA_MARKER)]);

        // Check envelope
        let pui_common = doc_obj
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            pui_common.get("marker").unwrap(),
            PUI_COMMON_METADATA_MARKER
        );
        assert_eq!(
            pui_common.get("uri").unwrap(),
            "/repositories/2/resources/100"
        );
        assert_eq!(pui_common.get("primary_type").unwrap(), "resource");

        // 1. Subjects
        let subjects = pui_common.get("subjects").unwrap().as_object().unwrap();
        assert_eq!(
            subjects.get("subjects").unwrap(),
            &json!(["Local History", "Archives"])
        );
        assert_eq!(
            subjects.get("subject_uris").unwrap(),
            &json!(["/subjects/1", "/subjects/2", "/subjects/3"])
        );

        // 2. Agents
        let agents = pui_common.get("agents").unwrap().as_object().unwrap();
        assert_eq!(
            agents.get("agents").unwrap(),
            &json!(["Doe, Jane", "Example Historical Society"])
        );
        assert_eq!(
            agents.get("agent_uris").unwrap(),
            &json!(["/agents/people/1", "/agents/corporate_entities/2"])
        );
        assert_eq!(
            agents.get("published_agents").unwrap(),
            &json!(["Doe, Jane"])
        );
        assert_eq!(
            agents.get("published_agent_uris").unwrap(),
            &json!(["/agents/people/1"])
        );
        assert_eq!(agents.get("creators").unwrap(), &json!(["Doe, Jane"]));

        // 3. Audit info
        let audit = pui_common.get("audit_info").unwrap().as_object().unwrap();
        assert_eq!(audit.get("created_by").unwrap(), "archivist");
        assert_eq!(audit.get("last_modified_by").unwrap(), "lead");
        assert_eq!(audit.get("user_mtime").unwrap(), "2026-10-19T12:00:00Z");
        assert_eq!(audit.get("system_mtime").unwrap(), "2026-10-19T12:00:01Z");
        assert_eq!(audit.get("create_time").unwrap(), "2026-10-01T00:00:00Z");

        // 4. Years
        let years = pui_common.get("years").unwrap().as_object().unwrap();
        assert_eq!(
            years.get("years").unwrap(),
            &json!(["1920", "1921", "1922", "1923", "1924", "1925", "1950"])
        );
        assert_eq!(years.get("year_sort").unwrap(), "19201950");
        // Inclusive date preference: only inclusive date is displayed
        assert_eq!(
            years.get("dates").unwrap(),
            &json!(["1920-01-01 - 1925-12-31"])
        );

        // 5. Level: otherlevel resolved to other_level
        let level = pui_common.get("level").unwrap().as_object().unwrap();
        assert_eq!(level.get("level").unwrap(), "subfonds");

        // 6. Summary: abstract note joined with newline
        let summary = pui_common.get("summary").unwrap().as_object().unwrap();
        assert_eq!(
            summary.get("summary").unwrap(),
            "First paragraph\nSecond paragraph"
        );

        // 7. Extents: whole extent preference
        let extents = pui_common.get("extents").unwrap().as_object().unwrap();
        assert_eq!(
            extents.get("extents").unwrap(),
            &json!(["1 --- linear_metres"])
        );
    }

    #[test]
    fn test_pui_common_metadata_common_only_request() {
        let record = json!({
            "uri": "/repositories/2/resources/201",
            "jsonmodel_type": "resource",
            "title": "Resource 201",
            "publish": true,
            "level": "collection"
        });

        let inp = IndexDocumentInput {
            uri: "/repositories/2/resources/201".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record,
            prepare_pui_common_metadata: true,
            prepare_pui_final: false,
            ..Default::default()
        };

        let doc = build_index_document(inp).expect("should build common-only doc");
        let doc_obj = doc.as_object().unwrap();

        // Exact marker list: only pui_common_metadata
        assert_eq!(
            doc_obj.get("_archivalist_rust_fields").unwrap(),
            &json!([PUI_COMMON_METADATA_MARKER])
        );
        assert!(doc_obj.contains_key(PUI_COMMON_DOCUMENTS_KEY));
        assert!(!doc_obj.contains_key(PUI_FINAL_DOCUMENTS_KEY));
        assert!(!doc_obj.contains_key(PUI_ROUTING_DOCUMENTS_KEY));
        assert!(!doc_obj.contains_key(PUI_NOTES_DOCUMENTS_KEY));
        // Verify no staff enrichment fields
        assert!(!doc_obj.contains_key("fullrecord"));
        assert!(!doc_obj.contains_key("fullrecord_published"));
    }

    #[test]
    fn test_pui_common_metadata_combined_request() {
        let record = json!({
            "uri": "/repositories/2/resources/202",
            "jsonmodel_type": "resource",
            "title": "Resource 202",
            "publish": true,
            "level": "collection"
        });

        let inp = IndexDocumentInput {
            uri: "/repositories/2/resources/202".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: record.clone(),
            pui_candidate_record: Some(record),
            prepare_pui_final: true,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };

        let doc = build_index_document(inp).expect("should build combined doc");
        let doc_obj = doc.as_object().unwrap();

        let markers = doc_obj
            .get("_archivalist_rust_fields")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            markers,
            &vec![
                json!(PUI_FINAL_DOCUMENT_METADATA_MARKER),
                json!(PUI_ROUTING_METADATA_MARKER),
                json!(PUI_NOTES_METADATA_MARKER),
                json!(PUI_COMMON_METADATA_MARKER),
            ]
        );
        assert!(doc_obj.contains_key(PUI_FINAL_DOCUMENTS_KEY));
        assert!(doc_obj.contains_key(PUI_ROUTING_DOCUMENTS_KEY));
        assert!(doc_obj.contains_key(PUI_NOTES_DOCUMENTS_KEY));
        assert!(doc_obj.contains_key(PUI_COMMON_DOCUMENTS_KEY));
    }

    #[test]
    fn test_pui_common_metadata_uses_direct_record_not_inherited_candidate() {
        let direct_record = json!({
            "uri": "/repositories/2/archival_objects/300",
            "jsonmodel_type": "archival_object",
            "title": "AO 300 Direct",
            "publish": true,
            "subjects": [
                {
                    "ref": "/subjects/direct",
                    "_resolved": {"title": "Direct Subject"}
                }
            ],
            "linked_agents": [
                {
                    "ref": "/agents/people/direct",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Direct Creator"},
                        "publish": true
                    }
                }
            ]
        });

        let candidate_record = json!({
            "uri": "/repositories/2/archival_objects/300",
            "jsonmodel_type": "archival_object",
            "title": "AO 300 Candidate",
            "publish": true,
            "subjects": [
                {
                    "ref": "/subjects/direct",
                    "_resolved": {"title": "Direct Subject"}
                },
                {
                    "ref": "/subjects/inherited",
                    "_resolved": {"title": "Inherited Subject"}
                }
            ],
            "linked_agents": [
                {
                    "ref": "/agents/people/direct",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Direct Creator"},
                        "publish": true
                    }
                },
                {
                    "ref": "/agents/people/inherited",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Inherited Creator"},
                        "publish": true
                    }
                }
            ]
        });

        let inp = IndexDocumentInput {
            uri: "/repositories/2/archival_objects/300".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record: direct_record,
            pui_candidate_record: Some(candidate_record),
            prepare_pui_final: true,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };

        let doc = build_index_document(inp).expect("should build doc");
        let doc_obj = doc.as_object().unwrap();
        let pui_common = doc_obj
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap();

        // Subjects in common metadata must come from direct record ONLY
        let subjects = pui_common.get("subjects").unwrap().as_object().unwrap();
        assert_eq!(
            subjects.get("subjects").unwrap(),
            &json!(["Direct Subject"])
        );
        assert_eq!(
            subjects.get("subject_uris").unwrap(),
            &json!(["/subjects/direct"])
        );

        // Agents in common metadata must come from direct record ONLY
        let agents = pui_common.get("agents").unwrap().as_object().unwrap();
        assert_eq!(agents.get("agents").unwrap(), &json!(["Direct Creator"]));
        assert_eq!(agents.get("creators").unwrap(), &json!(["Direct Creator"]));
    }

    #[test]
    fn test_pui_common_metadata_mode_conflicts() {
        let base_rec = json!({
            "uri": "/repositories/2/resources/500",
            "jsonmodel_type": "resource",
            "title": "Resource 500"
        });

        // 1. enrich + prepare_pui_common_metadata
        let inp1 = IndexDocumentInput {
            uri: "/repositories/2/resources/500".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: base_rec.clone(),
            enrich: true,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp1).is_err());

        // 2. prepare_pui_availability + prepare_pui_common_metadata
        let inp2 = IndexDocumentInput {
            uri: "/repositories/2/resources/500".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: base_rec.clone(),
            prepare_pui_availability: true,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp2).is_err());

        // 3. prepare_pui_auxiliary + prepare_pui_common_metadata
        let inp3 = IndexDocumentInput {
            uri: "/repositories/2/resources/500/ordered_records".to_string(),
            record_type: "resource_ordered_records".to_string(),
            repository: json!(2),
            record: base_rec,
            pui_parent_id: Some("/repositories/2/resources/500".to_string()),
            pui_auxiliary_json: Some("{}".to_string()),
            prepare_pui_auxiliary: true,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp3).is_err());
    }

    #[test]
    fn test_pui_common_metadata_exact_keys_and_non_mutation() {
        let source_record = json!({
            "uri": "/repositories/2/resources/600",
            "jsonmodel_type": "resource",
            "title": "Unmutated Test",
            "level": "collection",
            "dates": [{"date_type": "single", "begin": "1999"}]
        });
        let snapshot = source_record.clone();

        let inp = IndexDocumentInput {
            uri: "/repositories/2/resources/600".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: source_record,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };

        let doc = build_index_document(inp).expect("should build doc");
        let doc_obj = doc.as_object().unwrap();
        let pui_common = doc_obj
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap();

        // Exact keys
        let mut keys: Vec<&str> = pui_common.keys().map(String::as_str).collect();
        keys.sort();
        let mut expected = PUI_COMMON_FIELDS.to_vec();
        expected.sort();
        assert_eq!(keys, expected);

        // Meaningful non-mutation check: serialized record JSON in doc matches untouched snapshot
        let doc_json_str = doc_obj.get("json").unwrap().as_str().unwrap();
        let parsed_doc_json: Value = serde_json::from_str(doc_json_str).unwrap();
        assert_eq!(parsed_doc_json, snapshot);
    }

    #[test]
    fn test_pui_common_metadata_edge_cases_and_parity() {
        // 1. Creator absence: when no creator role exists, creators key is completely absent
        let no_creator_rec = json!({
            "uri": "/repositories/2/resources/601",
            "jsonmodel_type": "resource",
            "title": "No Creators",
            "linked_agents": [
                {
                    "ref": "/agents/people/9",
                    "role": "subject",
                    "_resolved": {
                        "display_name": {"sort_name": "Subject Agent"},
                        "publish": true
                    }
                }
            ]
        });
        let inp_no_creator = IndexDocumentInput {
            uri: "/repositories/2/resources/601".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: no_creator_rec,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        let doc = build_index_document(inp_no_creator).unwrap();
        let proj = doc
            .as_object()
            .unwrap()
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap();
        let agents = proj
            .as_object()
            .unwrap()
            .get("agents")
            .unwrap()
            .as_object()
            .unwrap();
        assert!(!agents.contains_key("creators"));
        assert_eq!(agents.get("agents").unwrap(), &json!(["Subject Agent"]));
        assert_eq!(
            agents.get("published_agents").unwrap(),
            &json!(["Subject Agent"])
        );

        // 2. Ruby truthiness in published agents: 0, "false", "" are truthy; false and null are falsy
        let truthy_rec = json!({
            "uri": "/repositories/2/resources/602",
            "jsonmodel_type": "resource",
            "title": "Truthiness Test",
            "linked_agents": [
                {
                    "ref": "/agents/people/1",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Agent Zero"},
                        "publish": 0
                    }
                },
                {
                    "ref": "/agents/people/2",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Agent StringFalse"},
                        "publish": "false"
                    }
                },
                {
                    "ref": "/agents/people/3",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Agent BoolFalse"},
                        "publish": false
                    }
                },
                {
                    "ref": "/agents/people/4",
                    "role": "creator",
                    "_resolved": {
                        "display_name": {"sort_name": "Agent Null"},
                        "publish": null
                    }
                }
            ]
        });
        let inp_truthy = IndexDocumentInput {
            uri: "/repositories/2/resources/602".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: truthy_rec,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        let doc_truthy = build_index_document(inp_truthy).unwrap();
        let agents_truthy = doc_truthy
            .as_object()
            .unwrap()
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap()
            .get("agents")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            agents_truthy.get("published_agents").unwrap(),
            &json!(["Agent Zero", "Agent StringFalse"])
        );

        // 3. Duplicate ordering preserved in subjects and agents
        let dups_rec = json!({
            "uri": "/repositories/2/resources/603",
            "jsonmodel_type": "resource",
            "title": "Duplicates Test",
            "subjects": [
                {"ref": "/subjects/1", "_resolved": {"title": "Topic A"}},
                {"ref": "/subjects/2", "_resolved": {"title": "Topic B"}},
                {"ref": "/subjects/1", "_resolved": {"title": "Topic A"}}
            ],
            "linked_agents": [
                {
                    "ref": "/agents/people/1",
                    "role": "creator",
                    "_resolved": {"display_name": {"sort_name": "Author 1"}, "publish": true}
                },
                {
                    "ref": "/agents/people/1",
                    "role": "subject",
                    "_resolved": {"display_name": {"sort_name": "Author 1"}, "publish": true}
                }
            ]
        });
        let inp_dups = IndexDocumentInput {
            uri: "/repositories/2/resources/603".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: dups_rec,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        let doc_dups = build_index_document(inp_dups).unwrap();
        let pui_dups = doc_dups
            .as_object()
            .unwrap()
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap();
        let subs_dups = pui_dups.get("subjects").unwrap().as_object().unwrap();
        assert_eq!(
            subs_dups.get("subjects").unwrap(),
            &json!(["Topic A", "Topic B", "Topic A"])
        );
        let ags_dups = pui_dups.get("agents").unwrap().as_object().unwrap();
        assert_eq!(
            ags_dups.get("agents").unwrap(),
            &json!(["Author 1", "Author 1"])
        );

        // 4. Scopecontent missing subnote content: empty string in join preserves separator
        let notes_rec = json!({
            "uri": "/repositories/2/resources/604",
            "jsonmodel_type": "resource",
            "title": "Notes Test",
            "notes": [
                {
                    "type": "scopecontent",
                    "subnotes": [
                        {"content": "Section 1"},
                        {"other": "no content field"},
                        {"content": "Section 2"}
                    ]
                }
            ]
        });
        let inp_notes = IndexDocumentInput {
            uri: "/repositories/2/resources/604".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: notes_rec,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        let doc_notes = build_index_document(inp_notes).unwrap();
        let summary_val = doc_notes
            .as_object()
            .unwrap()
            .get(PUI_COMMON_DOCUMENTS_KEY)
            .unwrap()
            .as_object()
            .unwrap()
            .get("summary")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            summary_val.get("summary").unwrap(),
            "Section 1\n\nSection 2"
        );

        // 5. Missing or invalid _resolved produces Err for fallback
        let bad_sub_rec = json!({
            "uri": "/repositories/2/resources/605",
            "jsonmodel_type": "resource",
            "title": "Bad Resolved",
            "subjects": [
                {"ref": "/subjects/1"} // missing _resolved
            ]
        });
        let inp_bad_sub = IndexDocumentInput {
            uri: "/repositories/2/resources/605".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: bad_sub_rec,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp_bad_sub).is_err());

        // 6. Unsupported recursive content in notes summary produces Err for fallback
        let bad_notes_rec = json!({
            "uri": "/repositories/2/resources/606",
            "jsonmodel_type": "resource",
            "title": "Recursive Note",
            "notes": [
                {
                    "type": "abstract",
                    "content": [{"nested": "object"}]
                }
            ]
        });
        let inp_bad_notes = IndexDocumentInput {
            uri: "/repositories/2/resources/606".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: bad_notes_rec,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp_bad_notes).is_err());
    }

    #[test]
    fn test_pui_common_metadata_mismatched_and_malformed_identity() {
        let record = json!({
            "uri": "/repositories/2/resources/700",
            "jsonmodel_type": "resource",
            "title": "Resource 700"
        });

        // Mismatched URI
        let inp1 = IndexDocumentInput {
            uri: "/repositories/2/resources/999".to_string(),
            record_type: "resource".to_string(),
            repository: json!(2),
            record: record.clone(),
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp1).is_err());

        // Mismatched record_type
        let inp2 = IndexDocumentInput {
            uri: "/repositories/2/resources/700".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(2),
            record,
            prepare_pui_common_metadata: true,
            ..Default::default()
        };
        assert!(build_index_document(inp2).is_err());
    }

    #[test]
    fn test_pui_final_non_alphabetical_nested_and_unicode_parity() {
        let raw_json = r#"{
            "z_custom": "custom_value",
            "uri": "/repositories/999/archival_objects/777",
            "title": "Émile Cartailhac 📚 — Société Géologique 中文",
            "suppressed": false,
            "publish": true,
            "jsonmodel_type": "archival_object",
            "repository": {"ref": "/repositories/999"},
            "notes": [
                {
                    "type": "scopecontent",
                    "publish": true,
                    "jsonmodel_type": "note_singlepart",
                    "content": ["Premier mémoire sur les cavernes pyrénéennes."]
                }
            ],
            "ancestors": [
                {
                    "ref": "/repositories/999/resources/10",
                    "level": "collection",
                    "_resolved": {"title": "Secret Ancestor Collection"}
                }
            ],
            "instances": [
                {
                    "instance_type": "mixed_materials",
                    "sub_container": {
                        "top_container": {
                            "ref": "/repositories/999/top_containers/501",
                            "_resolved": {
                                "z_custom": "keep_after_clean",
                                "display_string": "Boîte 501",
                                "internal_note": "TOP_SECRET_INTERNAL_NOTE_123",
                                "container_profile": {
                                    "_resolved": {
                                        "name": "Carton d'archives",
                                        "notes": "TOP_SECRET_PROFILE_NOTE_456"
                                    }
                                },
                                "a_custom": "keep_last"
                            }
                        }
                    }
                }
            ],
            "a_custom": "final_field"
        }"#;

        let candidate: Value = serde_json::from_str(raw_json).unwrap();
        let input = IndexDocumentInput {
            uri: "/repositories/999/archival_objects/777".to_string(),
            record_type: "archival_object".to_string(),
            repository: json!(999),
            record: candidate.clone(),
            prepare_pui_final: true,
            pui_candidate_record: Some(candidate),
            ..Default::default()
        };

        let doc = build_index_document(input).expect("must build successfully");
        let pui_final = &doc[PUI_FINAL_DOCUMENTS_KEY];
        let pui_json_str = pui_final["json"].as_str().unwrap();

        assert!(!pui_json_str.contains("ancestors"));
        assert!(!pui_json_str.contains("Secret Ancestor Collection"));
        assert!(!pui_json_str.contains("TOP_SECRET_INTERNAL_NOTE_123"));
        assert!(!pui_json_str.contains("TOP_SECRET_PROFILE_NOTE_456"));

        let parsed_clean: Value = serde_json::from_str(pui_json_str).unwrap();
        let obj = parsed_clean.as_object().unwrap();
        let keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "z_custom",
                "uri",
                "title",
                "suppressed",
                "publish",
                "jsonmodel_type",
                "repository",
                "notes",
                "instances",
                "a_custom"
            ]
        );

        let instances = obj["instances"].as_array().unwrap();
        let tc_resolved = instances[0]["sub_container"]["top_container"]["_resolved"]
            .as_object()
            .unwrap();
        let tc_keys: Vec<&str> = tc_resolved.keys().map(String::as_str).collect();
        assert_eq!(
            tc_keys,
            vec![
                "z_custom",
                "display_string",
                "container_profile",
                "a_custom"
            ]
        );

        let cp_resolved = tc_resolved["container_profile"]["_resolved"]
            .as_object()
            .unwrap();
        let cp_keys: Vec<&str> = cp_resolved.keys().map(String::as_str).collect();
        assert_eq!(cp_keys, vec!["name"]);

        let pub_strings = pui_final["fullrecord_published"].as_array().unwrap();
        let pub_vec: Vec<&str> = pub_strings.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(
            pub_vec,
            vec![
                "custom_value",
                "/repositories/999/archival_objects/777",
                "Émile Cartailhac 📚 — Société Géologique 中文",
                "final_field",
                "mixed_materials",
                "scopecontent",
                "Premier mémoire sur les cavernes pyrénéennes."
            ]
        );
    }

    #[test]
    fn test_record_inheritance_candidate_scalar_and_array_key_ordering() {
        let raw_record = r#"{
            "z_header": "top",
            "uri": "/repositories/999/archival_objects/888",
            "jsonmodel_type": "archival_object",
            "title": "Inheritance AO",
            "publish": true,
            "suppressed": false,
            "dates": [],
            "ancestors": [
                {
                    "ref": "/repositories/999/resources/10",
                    "level": "collection",
                    "_resolved": {
                        "dates": [
                            {"label": "creation", "expression": "1882", "begin": "1882-01-01"}
                        ],
                        "language": "fre"
                    }
                }
            ],
            "a_tail": "bottom"
        }"#;

        let record_val: Value = serde_json::from_str(raw_record).unwrap();
        let plan = RecordInheritancePlan {
            record_type: "archival_object".to_string(),
            rules: vec![
                RecordInheritanceRulePlan {
                    property: "dates".to_string(),
                    inherit_directly: true,
                    ancestor_index: 0,
                    ancestor_element_indices: Some(vec![0]),
                    inherited_value: json!([
                        {"label": "creation", "expression": "1882", "begin": "1882-01-01"}
                    ]),
                    inherited_props: InheritanceProps {
                        ref_val: Value::String("/repositories/999/resources/10".to_string()),
                        level: Value::String("Collection".to_string()),
                        direct: true,
                    },
                },
                RecordInheritanceRulePlan {
                    property: "language".to_string(),
                    inherit_directly: false,
                    ancestor_index: 0,
                    ancestor_element_indices: None,
                    inherited_value: Value::String("fre".to_string()),
                    inherited_props: InheritanceProps {
                        ref_val: Value::String("/repositories/999/resources/10".to_string()),
                        level: Value::String("Collection".to_string()),
                        direct: false,
                    },
                },
            ],
            composite_identifiers: Some(CompositeIdentifierPlan {
                pieces: vec!["COL-10".to_string(), "SER-888".to_string()],
                delimiter: ".".to_string(),
            }),
        };

        let merged =
            execute_record_inheritance_merge(&record_val, &plan).expect("merge must succeed");
        let keys: Vec<&str> = merged.keys().map(String::as_str).collect();

        assert_eq!(
            keys,
            vec![
                "z_header",
                "uri",
                "jsonmodel_type",
                "title",
                "publish",
                "suppressed",
                "dates",
                "ancestors",
                "a_tail",
                "language",
                "language_inherited",
                "_composite_identifier"
            ]
        );
    }

    #[test]
    fn test_index_document_input_rejects_missing_repository() {
        let json_str = r#"{
            "uri": "/repositories/4/resources/9/ordered_records",
            "record_type": "resource_ordered_records",
            "pui_parent_id": "/repositories/4/resources/9",
            "record": {"uris": ["/repositories/4/resources/9"]},
            "pui_auxiliary_json": "{\"uris\":[\"/repositories/4/resources/9\"]}",
            "prepare_pui_auxiliary": true
        }"#;
        let res: Result<IndexDocumentInput, _> = serde_json::from_str(json_str);
        assert!(
            res.is_err(),
            "missing ordinary repository field must reject"
        );
    }

    #[test]
    fn test_index_document_input_accepts_auxiliary_with_repository_global() {
        let json_str = r#"{
            "uri": "/repositories/4/resources/9/ordered_records",
            "record_type": "resource_ordered_records",
            "repository": "global",
            "pui_parent_id": "/repositories/4/resources/9",
            "record": {"uris": ["/repositories/4/resources/9"]},
            "pui_auxiliary_json": "{\"uris\":[\"/repositories/4/resources/9\"]}",
            "prepare_pui_auxiliary": true
        }"#;
        let input: IndexDocumentInput = serde_json::from_str(json_str)
            .expect("deserialization must succeed with repository global");
        assert_eq!(input.uri, "/repositories/4/resources/9/ordered_records");
        assert_eq!(input.repository, Value::String("global".to_string()));
        assert!(input.prepare_pui_auxiliary);
        let doc = build_index_document(input).expect("auxiliary build must succeed");
        assert_eq!(doc["id"], "/repositories/4/resources/9/ordered_records");
    }
}
