use anyhow::{bail, Context, Result};
use aspace_client::{
    build_index_document, build_index_document_batch, Client, IndexDocumentBatchRequest,
    IndexDocumentInput,
};
use serde_json::Value;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct StaffAoBuilderArgs {
    #[arg(short, long, alias = "repo-id", help = "Repository ID")]
    pub repo: u32,

    #[arg(
        long,
        value_delimiter = ',',
        help = "Comma-separated AO IDs to fetch and build"
    )]
    pub ids: Vec<u32>,

    #[arg(
        long,
        default_value_t = false,
        num_args = 0..=1,
        default_missing_value = "true",
        action = clap::ArgAction::Set,
        help = "Whether ARKs are enabled (default: false)"
    )]
    pub arks_enabled: bool,

    #[arg(
        long,
        default_value_t = true,
        num_args = 0..=1,
        default_missing_value = "true",
        action = clap::ArgAction::Set,
        help = "Strip private _archivalist_* capability markers (default: true)"
    )]
    pub strip_markers: bool,

    #[arg(long, help = "Path to write JSON output file (stdout if omitted)")]
    pub output: Option<PathBuf>,

    #[arg(
        long,
        help = "Isolated Solr base URL (e.g. http://127.0.0.1:18983/solr) for candidate write mode"
    )]
    pub solr_url: Option<String>,

    #[arg(
        long,
        help = "Target stage candidate core name (format: stage_staff_<nonce>_candidate)"
    )]
    pub candidate_core: Option<String>,
}

impl StaffAoBuilderArgs {
    pub fn validate(&self) -> Result<()> {
        if self.repo == 0 {
            bail!("repo ID must be greater than 0");
        }
        if self.ids.is_empty() {
            bail!("at least one AO ID must be specified");
        }
        if self.ids.len() > 100 {
            bail!(
                "cannot request more than 100 AO IDs per call (requested: {})",
                self.ids.len()
            );
        }
        match (&self.solr_url, &self.candidate_core) {
            (Some(url), Some(core)) => {
                aspace_client::isolated_solr_writer::validate_isolated_solr_base_url(url)?;
                aspace_client::isolated_solr_writer::validate_stage_candidate_core(core, None)?;
                if self.output.is_some() {
                    bail!(
                        "--output is not supported in write mode (--solr-url and --candidate-core)"
                    );
                }
            }
            (None, None) => {}
            _ => {
                bail!("--solr-url and --candidate-core must be specified together");
            }
        }
        Ok(())
    }
}

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct BuildIndexDocumentArgs {
    #[arg(
        long,
        help = "Path to input JSON file containing IndexDocumentInput or IndexDocumentBatchRequest (reads stdin if omitted)"
    )]
    pub input: Option<PathBuf>,

    #[arg(long, help = "Path to output JSON file (writes stdout if omitted)")]
    pub output: Option<PathBuf>,

    #[arg(
        long,
        default_value_t = false,
        help = "Strip private _archivalist_* capability markers"
    )]
    pub strip_markers: bool,
}

pub fn strip_archivalist_markers(doc: &mut Value) {
    if let Value::Object(map) = doc {
        map.retain(|k, _| !k.starts_with("_archivalist_"));
    }
}

pub async fn execute_staff_ao_builder(
    client: &Client,
    args: &StaffAoBuilderArgs,
) -> Result<Vec<Value>> {
    args.validate()?;

    let dynamic_enums = client
        .dynamic_enum_fields_from_schemas()
        .await
        .context("Failed to derive dynamic enum fields from /schemas")?;

    let raw_records = client
        .fetch_staff_archival_objects_by_ids(args.repo, &args.ids)
        .await
        .context("Failed to fetch staff archival objects from backend")?;

    if raw_records.len() != args.ids.len() {
        bail!(
            "Backend returned {} records, but {} were requested",
            raw_records.len(),
            args.ids.len()
        );
    }

    let repo_scope = Value::String(format!("/repositories/{}", args.repo));
    let mut inputs = Vec::with_capacity(raw_records.len());

    for record in raw_records {
        let uri = record
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Record missing valid string uri"))?
            .to_string();

        let input = IndexDocumentInput {
            uri,
            record_type: "archival_object".to_string(),
            repository: repo_scope.clone(),
            record,
            enrich: true,
            dynamic_enum_fields: dynamic_enums.clone(),
            arks_enabled: args.arks_enabled,
            ..Default::default()
        };
        inputs.push(input);
    }

    let batch = IndexDocumentBatchRequest { records: inputs };
    let resp = build_index_document_batch(batch)
        .map_err(|e| anyhow::anyhow!("Failed to build index document batch: {e}"))?;

    let mut docs = resp.documents;
    if args.strip_markers {
        for doc in &mut docs {
            strip_archivalist_markers(doc);
        }
    }

    if let (Some(solr_url), Some(candidate_core)) = (&args.solr_url, &args.candidate_core) {
        let mut writer = aspace_client::isolated_solr_writer::IsolatedSolrStaffAoWriter::new(
            solr_url,
            candidate_core,
        )
        .context("Failed to construct IsolatedSolrStaffAoWriter")?;
        writer
            .verify_target_core_ready()
            .await
            .context("Target candidate core readiness verification failed")?;
        writer
            .add_documents(docs.clone())
            .await
            .context("Failed to add documents to isolated Solr writer")?;
        writer
            .commit()
            .await
            .context("Failed to commit documents in isolated Solr writer")?;
    }

    Ok(docs)
}

pub fn write_file_mode_0600(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_filename = format!(".tmp_{}_{}_{}", std::process::id(), now_nanos, filename);
    let temp_path = parent.join(temp_filename);

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options
        .open(&temp_path)
        .with_context(|| format!("Failed to create temporary file at {:?}", temp_path))?;

    let write_res = (|| -> Result<()> {
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    drop(file);

    let link_res = std::fs::hard_link(&temp_path, path);
    let _ = std::fs::remove_file(&temp_path);

    if let Err(e) = link_res {
        bail!(
            "Failed to atomically publish file to {:?} without clobbering existing file: {}",
            path,
            e
        );
    }

    Ok(())
}

pub async fn run_staff_ao_builder(client: &Client, args: StaffAoBuilderArgs) -> Result<()> {
    run_staff_ao_builder_to_writer(client, args, &mut io::stdout()).await
}

pub async fn run_staff_ao_builder_to_writer<W: Write>(
    client: &Client,
    args: StaffAoBuilderArgs,
    writer: &mut W,
) -> Result<()> {
    let is_write_mode = args.solr_url.is_some() && args.candidate_core.is_some();
    let docs = execute_staff_ao_builder(client, &args).await?;

    if is_write_mode {
        // In write mode, stdout MUST contain only a compact non-sensitive status/count/core,
        // never raw document contents or output paths regardless of --output.
        let core = args.candidate_core.as_deref().unwrap_or_default();
        let summary = serde_json::json!({
            "status": "success",
            "count": docs.len(),
            "candidate_core": core,
        });
        writeln!(writer, "{}", serde_json::to_string(&summary)?)?;
        Ok(())
    } else {
        // Preview mode: explicit behavior to format documents to file or stdout
        let json_str = serde_json::to_string_pretty(&docs)?;
        match &args.output {
            Some(path) => write_file_mode_0600(path, json_str.as_bytes())?,
            None => writeln!(writer, "{json_str}")?,
        }
        Ok(())
    }
}

pub fn run_build_index_document(args: &BuildIndexDocumentArgs) -> Result<()> {
    let input_bytes = match &args.input {
        Some(path) => std::fs::read(path)?,
        None => {
            let mut buf = Vec::new();
            io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };

    let input_val: Value = serde_json::from_slice(&input_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid JSON input: {e}"))?;

    let mut output_val = if input_val.get("records").is_some() {
        let batch_req: IndexDocumentBatchRequest = serde_json::from_value(input_val)
            .map_err(|e| anyhow::anyhow!("Invalid IndexDocumentBatchRequest: {e}"))?;
        let resp = build_index_document_batch(batch_req)
            .map_err(|e| anyhow::anyhow!("Build error: {e}"))?;
        serde_json::to_value(resp)?
    } else {
        let doc_input: IndexDocumentInput = serde_json::from_value(input_val)
            .map_err(|e| anyhow::anyhow!("Invalid IndexDocumentInput: {e}"))?;
        let doc =
            build_index_document(doc_input).map_err(|e| anyhow::anyhow!("Build error: {e}"))?;
        doc
    };

    if args.strip_markers {
        if let Some(docs) = output_val
            .get_mut("documents")
            .and_then(Value::as_array_mut)
        {
            for doc in docs {
                strip_archivalist_markers(doc);
            }
        } else {
            strip_archivalist_markers(&mut output_val);
        }
    }

    let serialized = serde_json::to_string_pretty(&output_val)?;
    match &args.output {
        Some(path) => write_file_mode_0600(path, serialized.as_bytes())?,
        None => println!("{serialized}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use serde_json::json;

    #[test]
    fn test_staff_ao_builder_args_validation() {
        let invalid_repo = StaffAoBuilderArgs {
            repo: 0,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: None,
        };
        assert!(invalid_repo.validate().is_err());

        let empty_ids = StaffAoBuilderArgs {
            repo: 1,
            ids: vec![],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: None,
        };
        assert!(empty_ids.validate().is_err());

        let too_many_ids = StaffAoBuilderArgs {
            repo: 1,
            ids: (1..=101).collect(),
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: None,
        };
        assert!(too_many_ids.validate().is_err());

        let valid = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221, 2229],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: None,
        };
        assert!(valid.validate().is_ok());

        // Mismatched solr_url and candidate_core
        let mismatch1 = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: Some("http://127.0.0.1:8983/solr".to_string()),
            candidate_core: None,
        };
        assert!(mismatch1.validate().is_err());

        let mismatch2 = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: Some("stage_staff_nonce1_candidate".to_string()),
        };
        assert!(mismatch2.validate().is_err());

        // Non-loopback Solr URL refused
        let non_loopback = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: Some("http://remote-solr.com:8983/solr".to_string()),
            candidate_core: Some("stage_staff_nonce1_candidate".to_string()),
        };
        assert!(non_loopback.validate().is_err());

        // Forbidden core name refused
        let forbidden_core = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: Some("http://127.0.0.1:8983/solr".to_string()),
            candidate_core: Some("stage_staff_test_candidate".to_string()),
        };
        assert!(forbidden_core.validate().is_err());

        let valid_solr = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: Some("http://127.0.0.1:18983/solr".to_string()),
            candidate_core: Some("stage_staff_20261026_candidate".to_string()),
        };
        assert!(valid_solr.validate().is_ok());

        // Rejects --output in write mode
        let write_mode_with_output = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1],
            arks_enabled: false,
            strip_markers: true,
            output: Some(PathBuf::from("/tmp/leaked_records.json")),
            solr_url: Some("http://127.0.0.1:18983/solr".to_string()),
            candidate_core: Some("stage_staff_20261026_candidate".to_string()),
        };
        let err = write_mode_with_output.validate().unwrap_err();
        assert!(err.to_string().contains("--output"));
    }

    #[test]
    fn test_strip_archivalist_markers() {
        let mut doc = json!({
            "id": "/repositories/4/archival_objects/1",
            "title": "Test Title",
            "_archivalist_rust_fields": ["common_metadata"],
            "_archivalist_collection_management_documents": [],
            "repository": "/repositories/4"
        });

        strip_archivalist_markers(&mut doc);
        assert_eq!(doc["id"], "/repositories/4/archival_objects/1");
        assert_eq!(doc["title"], "Test Title");
        assert_eq!(doc["repository"], "/repositories/4");
        assert!(doc.get("_archivalist_rust_fields").is_none());
        assert!(doc
            .get("_archivalist_collection_management_documents")
            .is_none());
    }

    #[tokio::test]
    async fn test_execute_staff_ao_builder_offline() {
        let mut server = Server::new_async().await;

        let schemas_body = json!({
            "archival_object": {
                "type": "object",
                "properties": {
                    "level": {
                        "type": "string",
                        "dynamic_enum": "archival_record_level"
                    }
                }
            }
        });

        let _m_schemas = server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&schemas_body).unwrap())
            .create_async()
            .await;

        let ao_records = json!([
            {
                "uri": "/repositories/4/archival_objects/1221",
                "title": "Item 1221",
                "display_string": "Item 1221 display",
                "level": "item",
                "publish": true,
                "suppressed": false,
                "resource": {"ref": "/repositories/4/resources/10"},
                "ancestors": [{"ref": "/repositories/4/resources/10"}]
            }
        ]);

        let _m_ao = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/repositories/4/archival_objects\?.*id_set=1221.*".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&ao_records).unwrap())
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: None,
        };

        let docs = execute_staff_ao_builder(&client, &args)
            .await
            .expect("should execute staff ao builder");
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc["id"], "/repositories/4/archival_objects/1221");
        assert_eq!(doc["uri"], "/repositories/4/archival_objects/1221");
        assert_eq!(doc["primary_type"], "archival_object");
        assert_eq!(doc["repository"], "/repositories/4");
        assert_eq!(doc["title"], "Item 1221 display");
        assert_eq!(doc["resource"], "/repositories/4/resources/10");
        assert!(doc.get("_archivalist_rust_fields").is_none());
    }

    #[tokio::test]
    async fn test_execute_staff_ao_builder_with_solr_write_and_commit() {
        let mut server = Server::new_async().await;

        let schemas_payload = json!({
            "archival_object": {
                "properties": {
                    "level": {
                        "type": "string",
                        "dynamic_enum": "archival_record_level"
                    }
                }
            }
        });

        let _m_schemas = server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&schemas_payload).unwrap())
            .create_async()
            .await;

        let ao_records = json!([
            {
                "uri": "/repositories/4/archival_objects/1221",
                "title": "Item 1221",
                "display_string": "Item 1221 display",
                "level": "item",
                "publish": true,
                "suppressed": false,
                "resource": {"ref": "/repositories/4/resources/10"},
                "ancestors": [{"ref": "/repositories/4/resources/10"}]
            }
        ]);

        let _m_ao = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/repositories/4/archival_objects\?.*id_set=1221.*".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&ao_records).unwrap())
            .create_async()
            .await;

        let core_name = "stage_staff_20261026_casebuilderwrite_candidate";
        let status_path = format!("/admin/cores?action=STATUS&core={}&wt=json", core_name);
        let _m_status = server
            .mock("GET", status_path.as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"responseHeader":{{"status":0}},"status":{{"{}":{{"name":"{}"}}}}}}"#,
                core_name, core_name
            ))
            .create_async()
            .await;

        let select_path = format!("/{}/select?q=*:*&rows=0&wt=json", core_name);
        let _m_select = server
            .mock("GET", select_path.as_str())
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"response":{"numFound":0,"docs":[]}}"#)
            .create_async()
            .await;

        let update_path = format!("/{}/update", core_name);
        let _m_update = server
            .mock("POST", update_path.as_str())
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(2) // 1 for docs batch, 1 for commit
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("dummy-session".to_string());

        let args = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: Some(server.url()),
            candidate_core: Some(core_name.to_string()),
        };

        let docs = execute_staff_ao_builder(&client, &args)
            .await
            .expect("should write and commit to Solr candidate core");
        assert_eq!(docs.len(), 1);
    }

    #[tokio::test]
    async fn test_run_staff_ao_builder_write_mode_compact_stdout_no_record_leakage() {
        let mut server = Server::new_async().await;

        let schemas_payload = json!({
            "archival_object": {
                "properties": {
                    "level": {
                        "type": "string",
                        "dynamic_enum": "archival_record_level"
                    }
                }
            }
        });

        let _m_schemas = server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&schemas_payload).unwrap())
            .create_async()
            .await;

        let ao_records = json!([
            {
                "uri": "/repositories/4/archival_objects/1221",
                "title": "Confidential Record 1221",
                "display_string": "Sensitive Item Display 1221",
                "level": "item",
                "publish": false,
                "suppressed": false,
                "resource": {"ref": "/repositories/4/resources/10"},
                "ancestors": [{"ref": "/repositories/4/resources/10"}]
            }
        ]);

        let _m_ao = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/repositories/4/archival_objects\?.*id_set=1221.*".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&ao_records).unwrap())
            .create_async()
            .await;

        let core_name = "stage_staff_20261026_casebuilderwritemode_candidate";
        let status_path = format!("/admin/cores?action=STATUS&core={}&wt=json", core_name);
        let _m_status = server
            .mock("GET", status_path.as_str())
            .with_status(200)
            .with_body(format!(
                r#"{{"responseHeader":{{"status":0}},"status":{{"{}":{{"name":"{}"}}}}}}"#,
                core_name, core_name
            ))
            .create_async()
            .await;

        let select_path = format!("/{}/select?q=*:*&rows=0&wt=json", core_name);
        let _m_select = server
            .mock("GET", select_path.as_str())
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0},"response":{"numFound":0,"docs":[]}}"#)
            .create_async()
            .await;

        let update_path = format!("/{}/update", core_name);
        let _m_update = server
            .mock("POST", update_path.as_str())
            .with_status(200)
            .with_body(r#"{"responseHeader":{"status":0}}"#)
            .expect(2)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("dummy-session".to_string());

        let temp_dir = std::env::temp_dir().join(format!("test_ao_leakage_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let unwritten_output = temp_dir.join("should_not_be_written.json");

        // 1. First verify validate() rejects write mode when --output is provided
        let invalid_args = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221],
            arks_enabled: false,
            strip_markers: true,
            output: Some(unwritten_output.clone()),
            solr_url: Some(server.url()),
            candidate_core: Some(core_name.to_string()),
        };
        let val_err = invalid_args.validate().unwrap_err();
        assert!(val_err.to_string().contains("--output"));

        // 2. Run write mode without --output (standard write mode invocation)
        let valid_args = StaffAoBuilderArgs {
            output: None,
            ..invalid_args
        };

        let mut writer = Vec::new();
        run_staff_ao_builder_to_writer(&client, valid_args, &mut writer)
            .await
            .expect("should run staff ao builder in write mode");

        let stdout_str = String::from_utf8(writer).expect("valid utf8");

        // Stdout MUST parse as compact non-sensitive JSON status summary
        let summary_json: Value =
            serde_json::from_str(&stdout_str).expect("stdout must be valid json");
        assert_eq!(summary_json["status"], "success");
        assert_eq!(summary_json["count"], 1);
        assert_eq!(summary_json["candidate_core"], core_name);

        // Ensure NO raw record contents or fields leak to stdout
        assert!(!stdout_str.contains("Confidential Record 1221"));
        assert!(!stdout_str.contains("Sensitive Item Display 1221"));
        assert!(!stdout_str.contains("/repositories/4/archival_objects/1221"));
        assert!(!stdout_str.contains("/repositories/4/resources/10"));
        assert!(!stdout_str.contains("display_string"));
        assert!(!stdout_str.contains("ancestors"));
        assert!(!stdout_str.contains("should_not_be_written"));

        // Ensure no file was created at the output path (no leakage)
        assert!(
            !unwritten_output.exists(),
            "Output file must NOT be created in write mode"
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_run_staff_ao_builder_preview_mode_explicit_behavior() {
        let mut server = Server::new_async().await;

        let schemas_payload = json!({
            "archival_object": {
                "properties": {
                    "level": {
                        "type": "string",
                        "dynamic_enum": "archival_record_level"
                    }
                }
            }
        });

        let _m_schemas = server
            .mock("GET", "/schemas")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&schemas_payload).unwrap())
            .create_async()
            .await;

        let ao_records = json!([
            {
                "uri": "/repositories/4/archival_objects/1221",
                "title": "Preview Record 1221",
                "display_string": "Preview Display 1221",
                "level": "item",
                "publish": true,
                "suppressed": false,
                "resource": {"ref": "/repositories/4/resources/10"},
                "ancestors": [{"ref": "/repositories/4/resources/10"}]
            }
        ]);

        let _m_ao = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/repositories/4/archival_objects\?.*id_set=1221.*".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&ao_records).unwrap())
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("dummy-session".to_string());

        // 1. Preview mode without --output prints formatted documents to stdout
        let args_stdout = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221],
            arks_enabled: false,
            strip_markers: true,
            output: None,
            solr_url: None,
            candidate_core: None,
        };

        let mut stdout_buf = Vec::new();
        run_staff_ao_builder_to_writer(&client, args_stdout, &mut stdout_buf)
            .await
            .expect("should run preview mode to stdout");

        let stdout_str = String::from_utf8(stdout_buf).expect("valid utf8");
        assert!(stdout_str.contains("/repositories/4/archival_objects/1221"));
        assert!(stdout_str.contains("Preview Display 1221"));

        // 2. Preview mode with --output writes 0600 file and does not print to stdout
        let temp_dir = std::env::temp_dir().join(format!("test_ao_preview_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let output_file = temp_dir.join("preview_out.json");

        let args_file = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221],
            arks_enabled: false,
            strip_markers: true,
            output: Some(output_file.clone()),
            solr_url: None,
            candidate_core: None,
        };

        let mut empty_buf = Vec::new();
        run_staff_ao_builder_to_writer(&client, args_file, &mut empty_buf)
            .await
            .expect("should run preview mode to file");

        assert!(
            empty_buf.is_empty(),
            "Stdout must be empty when --output is specified in preview mode"
        );
        assert!(
            output_file.exists(),
            "Output file must be written in preview mode"
        );
        let file_content = std::fs::read_to_string(&output_file).expect("read file");
        assert!(file_content.contains("/repositories/4/archival_objects/1221"));
        assert!(file_content.contains("Preview Display 1221"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_write_file_mode_0600_permissions_and_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        let temp_dir =
            std::env::temp_dir().join(format!("test_ao_builder_mode_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let target_file = temp_dir.join("test_ao_doc.json");
        let test_data = b"{\"test\": \"mode0600\"}";
        let res = write_file_mode_0600(&target_file, test_data);
        assert!(res.is_ok(), "write_file_mode_0600 failed: {:?}", res);

        let metadata = std::fs::metadata(&target_file).expect("metadata should exist");
        let permissions = metadata.permissions();
        assert_eq!(
            permissions.mode() & 0o777,
            0o600,
            "Expected file mode 0600, got {:o}",
            permissions.mode() & 0o777
        );
        let read_back = std::fs::read(&target_file).expect("should read back target");
        assert_eq!(read_back, test_data);

        // Verify no leftover .tmp_* files
        let entries = std::fs::read_dir(&temp_dir).expect("should read temp dir");
        for entry in entries {
            let entry = entry.expect("valid entry");
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with(".tmp_"),
                "Found unexpected leftover temp file: {}",
                name
            );
        }

        let _ = std::fs::remove_file(&target_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_write_file_mode_0600_no_clobber_existing_target() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_ao_builder_noclobber_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let target_file = temp_dir.join("existing_user_doc.json");
        let original_data = b"ORIGINAL_USER_CONTENT_PRESERVED";
        std::fs::write(&target_file, original_data).expect("should create pre-existing file");

        let overwrite_data = b"OVERWRITE_ATTEMPT_DATA";
        let res = write_file_mode_0600(&target_file, overwrite_data);
        assert!(
            res.is_err(),
            "write_file_mode_0600 should fail when target file already exists"
        );

        // Verify pre-existing content is untouched
        let content_after = std::fs::read(&target_file).expect("target should still exist");
        assert_eq!(
            content_after, original_data,
            "Target file content was modified despite failure!"
        );

        // Verify no leftover temp files
        let entries = std::fs::read_dir(&temp_dir).expect("should read temp dir");
        for entry in entries {
            let entry = entry.expect("valid entry");
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with(".tmp_"),
                "Found unexpected leftover temp file: {}",
                name
            );
        }

        let _ = std::fs::remove_file(&target_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_write_file_mode_0600_failure_cleanup_nonexistent_dir() {
        let non_existent_dir = std::env::temp_dir()
            .join(format!("nonexistent_dir_{}", std::process::id()))
            .join("sub");
        let target_file = non_existent_dir.join("test_ao_doc.json");
        let res = write_file_mode_0600(&target_file, b"test");
        assert!(res.is_err());
        assert!(!target_file.exists());
    }
}
