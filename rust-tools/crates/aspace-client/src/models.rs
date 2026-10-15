use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Repository {
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct Date {
    pub expression: Option<String>,
    pub begin: Option<String>,
    pub end: Option<String>,
    pub date_type: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct Extent {
    pub number: Option<String>,
    pub extent_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct Resource {
    pub uri: String,
    pub title: Option<String>,
    pub system_mtime: Option<String>,
    pub user_mtime: Option<String>,
    #[serde(default)]
    pub publish: bool,
    #[serde(default)]
    pub dates: Vec<Date>,
    #[serde(default)]
    pub extents: Vec<Extent>,
    pub jsonmodel_type: Option<String>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct Ref {
    #[serde(rename = "ref", alias = "ref_uri")]
    pub ref_uri: String,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct AncestorRef {
    #[serde(rename = "ref", alias = "ref_uri", default)]
    pub ref_uri: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct LinkedAgentRef {
    #[serde(rename = "ref", alias = "ref_uri", default)]
    pub ref_uri: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub relator: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub terms: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct ArchivalObject {
    pub uri: String,
    pub title: Option<String>,
    pub display_string: Option<String>,
    pub system_mtime: Option<String>,
    pub user_mtime: Option<String>,
    #[serde(default)]
    pub publish: bool,
    #[serde(default)]
    pub dates: Vec<Date>,
    pub resource: Option<Ref>,
    #[serde(default)]
    pub parent: Option<Ref>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub ancestors: Vec<AncestorRef>,
    #[serde(default)]
    pub jsonmodel_type: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub component_id: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub subjects: Vec<Ref>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub linked_agents: Vec<LinkedAgentRef>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct FileVersion {
    pub file_uri: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
pub struct DigitalObject {
    pub uri: String,
    pub title: Option<String>,
    pub digital_object_id: Option<String>,
    pub system_mtime: Option<String>,
    pub user_mtime: Option<String>,
    #[serde(default)]
    pub publish: bool,
    #[serde(default)]
    pub file_versions: Vec<FileVersion>,
    #[serde(default)]
    pub jsonmodel_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archival_object_serde_defaults() {
        let json = r#"{"uri": "/repositories/2/archival_objects/1"}"#;
        let ao: ArchivalObject = serde_json::from_str(json).unwrap();
        assert_eq!(ao.uri, "/repositories/2/archival_objects/1");
        assert_eq!(ao.level, None);
        assert_eq!(ao.ref_id, None);
        assert_eq!(ao.component_id, None);
        assert_eq!(ao.display_string, None);
        assert_eq!(ao.dates, Vec::<Date>::new());
    }

    #[test]
    fn test_archival_object_serde_with_optional_fields() {
        let json = r#"{
            "uri": "/repositories/2/archival_objects/2",
            "title": "Subseries 1",
            "display_string": "Subseries 1, 1970-1980",
            "level": "subseries",
            "ref_id": "ao_ref_789",
            "component_id": "SS-01"
        }"#;
        let ao: ArchivalObject = serde_json::from_str(json).unwrap();
        assert_eq!(ao.uri, "/repositories/2/archival_objects/2");
        assert_eq!(ao.title.as_deref(), Some("Subseries 1"));
        assert_eq!(ao.display_string.as_deref(), Some("Subseries 1, 1970-1980"));
        assert_eq!(ao.level.as_deref(), Some("subseries"));
        assert_eq!(ao.ref_id.as_deref(), Some("ao_ref_789"));
        assert_eq!(ao.component_id.as_deref(), Some("SS-01"));
    }

    #[test]
    fn test_archival_object_serde_subjects_and_linked_agents() {
        let json = r#"{
            "uri": "/repositories/2/archival_objects/3",
            "subjects": [
                {"ref": "/subjects/10"},
                {"ref": "/subjects/20"}
            ],
            "linked_agents": [
                {
                    "ref": "/agents/people/1",
                    "role": "creator",
                    "relator": "aut",
                    "title": "Author Person",
                    "terms": [{"term": "term1"}]
                },
                {
                    "ref": "/agents/corporate_entities/2",
                    "role": "subject"
                }
            ]
        }"#;
        let ao: ArchivalObject = serde_json::from_str(json).unwrap();
        assert_eq!(ao.subjects.len(), 2);
        assert_eq!(ao.subjects[0].ref_uri, "/subjects/10");
        assert_eq!(ao.subjects[1].ref_uri, "/subjects/20");

        assert_eq!(ao.linked_agents.len(), 2);
        let a0 = &ao.linked_agents[0];
        assert_eq!(a0.ref_uri.as_deref(), Some("/agents/people/1"));
        assert_eq!(a0.role.as_deref(), Some("creator"));
        assert_eq!(a0.relator.as_deref(), Some("aut"));
        assert_eq!(a0.title.as_deref(), Some("Author Person"));
        assert_eq!(a0.terms.len(), 1);

        let a1 = &ao.linked_agents[1];
        assert_eq!(a1.ref_uri.as_deref(), Some("/agents/corporate_entities/2"));
        assert_eq!(a1.role.as_deref(), Some("subject"));
        assert_eq!(a1.relator, None);
        assert_eq!(a1.title, None);
        assert!(a1.terms.is_empty());
    }

    #[test]
    fn test_archival_object_serde_null_subjects_and_linked_agents() {
        let json = r#"{
            "uri": "/repositories/2/archival_objects/4",
            "subjects": null,
            "linked_agents": null
        }"#;
        let ao: ArchivalObject = serde_json::from_str(json).unwrap();
        assert!(ao.subjects.is_empty());
        assert!(ao.linked_agents.is_empty());
    }

    #[test]
    fn test_archival_object_serde_parent_and_ancestors() {
        let json = r#"{
            "uri": "/repositories/2/archival_objects/5",
            "parent": {"ref": "/repositories/2/archival_objects/4"},
            "ancestors": [
                {
                    "ref": "/repositories/2/archival_objects/3",
                    "level": "series",
                    "title": "Series 1"
                },
                {
                    "ref_uri": "/repositories/2/archival_objects/4",
                    "level": "subseries"
                },
                {
                    "title": "Untitled node"
                }
            ]
        }"#;
        let ao: ArchivalObject = serde_json::from_str(json).unwrap();
        assert_eq!(
            ao.parent,
            Some(Ref {
                ref_uri: "/repositories/2/archival_objects/4".to_string()
            })
        );
        assert_eq!(ao.ancestors.len(), 3);
        assert_eq!(
            ao.ancestors[0],
            AncestorRef {
                ref_uri: Some("/repositories/2/archival_objects/3".to_string()),
                level: Some("series".to_string()),
                title: Some("Series 1".to_string()),
            }
        );
        assert_eq!(
            ao.ancestors[1],
            AncestorRef {
                ref_uri: Some("/repositories/2/archival_objects/4".to_string()),
                level: Some("subseries".to_string()),
                title: None,
            }
        );
        assert_eq!(
            ao.ancestors[2],
            AncestorRef {
                ref_uri: None,
                level: None,
                title: Some("Untitled node".to_string()),
            }
        );
    }

    #[test]
    fn test_archival_object_serde_null_and_missing_hierarchy() {
        let json = r#"{
            "uri": "/repositories/2/archival_objects/6",
            "parent": null,
            "ancestors": null
        }"#;
        let ao: ArchivalObject = serde_json::from_str(json).unwrap();
        assert_eq!(ao.parent, None);
        assert!(ao.ancestors.is_empty());

        let json_omitted = r#"{"uri": "/repositories/2/archival_objects/7"}"#;
        let ao_omitted: ArchivalObject = serde_json::from_str(json_omitted).unwrap();
        assert_eq!(ao_omitted.parent, None);
        assert!(ao_omitted.ancestors.is_empty());
    }

    #[test]
    fn test_archival_object_serde_position() {
        let json_with_pos = r#"{
            "uri": "/repositories/2/archival_objects/1",
            "position": 5
        }"#;
        let ao_with_pos: ArchivalObject = serde_json::from_str(json_with_pos).unwrap();
        assert_eq!(ao_with_pos.position, Some(5));

        let json_zero_pos = r#"{
            "uri": "/repositories/2/archival_objects/2",
            "position": 0
        }"#;
        let ao_zero_pos: ArchivalObject = serde_json::from_str(json_zero_pos).unwrap();
        assert_eq!(ao_zero_pos.position, Some(0));

        let json_null_pos = r#"{
            "uri": "/repositories/2/archival_objects/3",
            "position": null
        }"#;
        let ao_null_pos: ArchivalObject = serde_json::from_str(json_null_pos).unwrap();
        assert_eq!(ao_null_pos.position, None);

        let json_no_pos = r#"{
            "uri": "/repositories/2/archival_objects/4"
        }"#;
        let ao_no_pos: ArchivalObject = serde_json::from_str(json_no_pos).unwrap();
        assert_eq!(ao_no_pos.position, None);
    }
}
