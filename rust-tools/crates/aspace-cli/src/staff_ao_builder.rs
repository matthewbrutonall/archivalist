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
    let docs = execute_staff_ao_builder(client, &args).await?;
    let json_str = serde_json::to_string_pretty(&docs)?;
    match &args.output {
        Some(path) => write_file_mode_0600(path, json_str.as_bytes())?,
        None => println!("{json_str}"),
    }
    Ok(())
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
        };
        assert!(invalid_repo.validate().is_err());

        let empty_ids = StaffAoBuilderArgs {
            repo: 1,
            ids: vec![],
            arks_enabled: false,
            strip_markers: true,
            output: None,
        };
        assert!(empty_ids.validate().is_err());

        let too_many_ids = StaffAoBuilderArgs {
            repo: 1,
            ids: (1..=101).collect(),
            arks_enabled: false,
            strip_markers: true,
            output: None,
        };
        assert!(too_many_ids.validate().is_err());

        let valid = StaffAoBuilderArgs {
            repo: 4,
            ids: vec![1221, 2229],
            arks_enabled: false,
            strip_markers: true,
            output: None,
        };
        assert!(valid.validate().is_ok());
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
