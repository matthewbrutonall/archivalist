use crate::models::{ArchivalObject, DigitalObject, Resource};
use serde::Serialize;

#[derive(Debug, Serialize, Default, PartialEq)]
pub struct ValidationSummary {
    pub total_records: usize,
    pub valid_records: usize,
    pub missing_title: usize,
    pub missing_dates: usize,
    pub missing_extents: usize,
    pub missing_digital_object_id: usize,
    pub missing_file_versions: usize,
    pub unpublished: usize,
}

#[derive(Debug, PartialEq, Serialize, Clone)]
pub enum ValidationIssue {
    MissingTitle,
    MissingDates,
    MissingExtents,
    MissingDigitalObjectId,
    MissingFileVersions,
}

#[derive(Debug, PartialEq, Serialize, Clone)]
pub enum ValidationSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Serialize)]
pub struct ValidationIssueRow {
    pub repo_id: u32,
    pub uri: String,
    pub title: Option<String>,
    pub issue: ValidationIssue,
    pub severity: ValidationSeverity,
    pub parent_resource: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct ValidationMetrics {
    pub endpoint_url: String,
    pub record_type: String,
    pub repo_id: u32,
    pub total_ids: usize,
    pub total_batches: usize,
    pub batch_size: usize,
    pub elapsed_time_sec: f64,
    pub batch_elapsed_time_sec: f64,
    pub records_per_sec: f64,
    pub output_mode: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub repo_id: u32,
    pub summary: ValidationSummary,
    pub metrics: ValidationMetrics,
    pub issues: Vec<ValidationIssueRow>,
}

pub struct ValidationResult {
    pub is_valid: bool,
    pub issues: Vec<(ValidationIssue, ValidationSeverity)>,
}

fn has_valid_dates(dates: &[crate::models::Date]) -> bool {
    dates.iter().any(|d| {
        d.expression.as_ref().is_some_and(|s| !s.trim().is_empty())
            || d.begin.as_ref().is_some_and(|s| !s.trim().is_empty())
            || d.end.as_ref().is_some_and(|s| !s.trim().is_empty())
    })
}

pub fn validate_resource(r: &Resource, summary: &mut ValidationSummary) -> ValidationResult {
    summary.total_records += 1;

    let mut is_valid = true;
    let mut issues = Vec::new();

    if !r.publish {
        summary.unpublished += 1;
    }

    if r.title.as_ref().is_none_or(|t| t.trim().is_empty()) {
        summary.missing_title += 1;
        issues.push((ValidationIssue::MissingTitle, ValidationSeverity::Error));
        is_valid = false;
    }

    if !has_valid_dates(&r.dates) {
        summary.missing_dates += 1;
        issues.push((ValidationIssue::MissingDates, ValidationSeverity::Error));
        is_valid = false;
    }

    if r.extents.is_empty() {
        summary.missing_extents += 1;
        issues.push((ValidationIssue::MissingExtents, ValidationSeverity::Error));
        is_valid = false;
    }

    if is_valid {
        summary.valid_records += 1;
    }

    ValidationResult { is_valid, issues }
}

pub fn validate_archival_object(
    ao: &ArchivalObject,
    summary: &mut ValidationSummary,
) -> ValidationResult {
    summary.total_records += 1;

    let mut is_valid = true;
    let mut issues = Vec::new();

    if !ao.publish {
        summary.unpublished += 1;
    }

    let has_title = ao.title.as_ref().is_some_and(|t| !t.trim().is_empty());
    let has_display_string = ao
        .display_string
        .as_ref()
        .is_some_and(|d| !d.trim().is_empty());

    if !has_title && !has_display_string {
        summary.missing_title += 1;
        issues.push((ValidationIssue::MissingTitle, ValidationSeverity::Error));
        is_valid = false;
    }

    if !has_valid_dates(&ao.dates) {
        summary.missing_dates += 1;
        issues.push((ValidationIssue::MissingDates, ValidationSeverity::Error));
        is_valid = false;
    }

    if is_valid {
        summary.valid_records += 1;
    }

    ValidationResult { is_valid, issues }
}

pub fn validate_digital_object(
    do_obj: &DigitalObject,
    summary: &mut ValidationSummary,
) -> ValidationResult {
    summary.total_records += 1;

    let mut is_valid = true;
    let mut issues = Vec::new();

    if !do_obj.publish {
        summary.unpublished += 1;
    }

    if do_obj.title.as_ref().is_none_or(|t| t.trim().is_empty()) {
        summary.missing_title += 1;
        issues.push((ValidationIssue::MissingTitle, ValidationSeverity::Error));
        is_valid = false;
    }

    if do_obj
        .digital_object_id
        .as_ref()
        .is_none_or(|id| id.trim().is_empty())
    {
        summary.missing_digital_object_id += 1;
        issues.push((
            ValidationIssue::MissingDigitalObjectId,
            ValidationSeverity::Error,
        ));
        is_valid = false;
    }

    if do_obj.file_versions.is_empty() {
        summary.missing_file_versions += 1;
        issues.push((
            ValidationIssue::MissingFileVersions,
            ValidationSeverity::Error,
        ));
        is_valid = false;
    }

    if is_valid {
        summary.valid_records += 1;
    }

    ValidationResult { is_valid, issues }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Date, Extent, FileVersion, Ref};

    #[test]
    fn test_valid_resource() {
        let r = Resource {
            uri: "/r/1".into(),
            title: Some("Valid Title".into()),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date {
                expression: Some("1990".into()),
                ..Date::default()
            }],
            extents: vec![Extent::default()],
            jsonmodel_type: None,
        };
        let mut summary = ValidationSummary::default();
        let res = validate_resource(&r, &mut summary);
        assert!(res.is_valid);
    }

    #[test]
    fn test_resource_missing_title() {
        let r = Resource {
            uri: "/r/1".into(),
            title: Some("   ".into()),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date {
                expression: Some("1990".into()),
                ..Date::default()
            }],
            extents: vec![Extent::default()],
            jsonmodel_type: None,
        };
        let mut summary = ValidationSummary::default();
        let res = validate_resource(&r, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_title, 1);
        assert_eq!(res.issues[0].0, ValidationIssue::MissingTitle);
    }

    #[test]
    fn test_resource_missing_dates_and_extents() {
        let r = Resource {
            uri: "/r/1".into(),
            title: Some("Title".into()),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![],
            extents: vec![],
            jsonmodel_type: None,
        };
        let mut summary = ValidationSummary::default();
        let res = validate_resource(&r, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_dates, 1);
        assert_eq!(summary.missing_extents, 1);
    }

    #[test]
    fn test_resource_unpublished_is_not_invalid() {
        let r = Resource {
            uri: "/r/1".into(),
            title: Some("Title".into()),
            system_mtime: None,
            user_mtime: None,
            publish: false,
            dates: vec![Date {
                expression: Some("1990".into()),
                ..Date::default()
            }],
            extents: vec![Extent::default()],
            jsonmodel_type: None,
        };
        let mut summary = ValidationSummary::default();
        let res = validate_resource(&r, &mut summary);
        assert!(res.is_valid);
        assert_eq!(summary.valid_records, 1);
        assert_eq!(summary.unpublished, 1);
    }

    #[test]
    fn test_valid_ao() {
        let ao = ArchivalObject {
            uri: "/ao/1".into(),
            title: Some("Valid AO".into()),
            display_string: None,
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date {
                expression: Some("1990".into()),
                ..Date::default()
            }],
            jsonmodel_type: None,
            resource: Some(Ref {
                ref_uri: "/r/1".into(),
            }),
            ..Default::default()
        };
        let mut summary = ValidationSummary::default();
        let res = validate_archival_object(&ao, &mut summary);
        assert!(res.is_valid);
    }

    #[test]
    fn test_ao_missing_title() {
        let ao = ArchivalObject {
            uri: "/ao/1".into(),
            title: None,
            display_string: None,
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date {
                expression: Some("1990".into()),
                ..Date::default()
            }],
            resource: None,
            jsonmodel_type: None,
            ..Default::default()
        };
        let mut summary = ValidationSummary::default();
        let res = validate_archival_object(&ao, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_title, 1);
    }

    #[test]
    fn test_resource_date_default_invalid() {
        let r = Resource {
            uri: "/r/1".into(),
            title: Some("Valid Title".into()),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date::default()],
            extents: vec![Extent::default()],
            jsonmodel_type: None,
        };
        let mut summary = ValidationSummary::default();
        let res = validate_resource(&r, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_dates, 1);
        assert!(res
            .issues
            .iter()
            .any(|i| i.0 == ValidationIssue::MissingDates));
    }

    #[test]
    fn test_ao_date_default_invalid() {
        let ao = ArchivalObject {
            uri: "/ao/1".into(),
            title: Some("Valid AO".into()),
            display_string: None,
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date::default()],
            jsonmodel_type: None,
            resource: Some(Ref {
                ref_uri: "/r/1".into(),
            }),
            ..Default::default()
        };
        let mut summary = ValidationSummary::default();
        let res = validate_archival_object(&ao, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_dates, 1);
        assert!(res
            .issues
            .iter()
            .any(|i| i.0 == ValidationIssue::MissingDates));
    }

    #[test]
    fn test_ao_whitespace_only_title_display_string_invalid() {
        let ao = ArchivalObject {
            uri: "/ao/1".into(),
            title: Some("   ".into()),
            display_string: Some(
                "
"
                .into(),
            ),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            dates: vec![Date {
                expression: Some("1990".into()),
                ..Date::default()
            }],
            jsonmodel_type: None,
            resource: Some(Ref {
                ref_uri: "/r/1".into(),
            }),
            ..Default::default()
        };
        let mut summary = ValidationSummary::default();
        let res = validate_archival_object(&ao, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_title, 1);
        assert!(res
            .issues
            .iter()
            .any(|i| i.0 == ValidationIssue::MissingTitle));
    }

    #[test]
    fn test_valid_digital_object() {
        let do_obj = DigitalObject {
            uri: "/do/1".into(),
            title: Some("Valid DO".into()),
            digital_object_id: Some("do_123".into()),
            system_mtime: None,
            user_mtime: None,
            publish: true,
            file_versions: vec![FileVersion {
                file_uri: Some("file://1".into()),
            }],
            jsonmodel_type: Some("digital_object".into()),
        };
        let mut summary = ValidationSummary::default();
        let res = validate_digital_object(&do_obj, &mut summary);
        assert!(res.is_valid);
    }

    #[test]
    fn test_do_missing_id_and_files() {
        let do_obj = DigitalObject {
            uri: "/do/1".into(),
            title: Some("Valid DO".into()),
            digital_object_id: None,
            system_mtime: None,
            user_mtime: None,
            publish: true,
            file_versions: vec![],
            jsonmodel_type: Some("digital_object".into()),
        };
        let mut summary = ValidationSummary::default();
        let res = validate_digital_object(&do_obj, &mut summary);
        assert!(!res.is_valid);
        assert_eq!(summary.missing_digital_object_id, 1);
        assert_eq!(summary.missing_file_versions, 1);
    }

    #[test]
    fn test_report_serialization() {
        let report = ValidationReport {
            repo_id: 2,
            summary: ValidationSummary {
                total_records: 1,
                valid_records: 0,
                missing_title: 1,
                missing_dates: 0,
                missing_extents: 0,
                missing_digital_object_id: 0,
                missing_file_versions: 0,
                unpublished: 0,
            },
            metrics: ValidationMetrics {
                endpoint_url: "http://localhost:8089".into(),
                record_type: "resource".into(),
                repo_id: 2,
                total_ids: 1,
                total_batches: 1,
                batch_size: 50,
                elapsed_time_sec: 0.1,
                batch_elapsed_time_sec: 0.1,
                records_per_sec: 10.0,
                output_mode: "json".into(),
            },
            issues: vec![ValidationIssueRow {
                repo_id: 2,
                uri: "/r/1".into(),
                title: None,
                issue: ValidationIssue::MissingTitle,
                severity: ValidationSeverity::Error,
                parent_resource: None,
            }],
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""repo_id":2"#));
        assert!(json.contains(r#""MissingTitle""#));
        assert!(json.contains(r#""Error""#));
    }

    #[test]
    fn test_csv_serialization_empty() {
        let mut wtr = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(vec![]);
        wtr.write_record([
            "repo_id",
            "uri",
            "title",
            "issue",
            "severity",
            "parent_resource",
        ])
        .unwrap();
        let out = String::from_utf8(wtr.into_inner().unwrap()).unwrap();
        assert_eq!(out, "repo_id,uri,title,issue,severity,parent_resource\n");
    }

    #[test]
    fn test_csv_serialization_with_issue() {
        let mut wtr = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(vec![]);
        wtr.write_record([
            "repo_id",
            "uri",
            "title",
            "issue",
            "severity",
            "parent_resource",
        ])
        .unwrap();

        let issue = ValidationIssueRow {
            repo_id: 2,
            uri: "/r/1".into(),
            title: Some("Title, with comma".into()),
            issue: ValidationIssue::MissingDates,
            severity: ValidationSeverity::Error,
            parent_resource: Some("/r/5".into()),
        };

        wtr.serialize(issue).unwrap();
        let out = String::from_utf8(wtr.into_inner().unwrap()).unwrap();

        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "repo_id,uri,title,issue,severity,parent_resource");
        assert_eq!(
            lines[1],
            "2,/r/1,\"Title, with comma\",MissingDates,Error,/r/5"
        );
    }
}
