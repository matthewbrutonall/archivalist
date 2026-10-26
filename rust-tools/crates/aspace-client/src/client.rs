use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;

use crate::error::{ASpaceError, Result};
use crate::models::{ArchivalObject, Repository, Resource};
use crate::schema_enums::extract_dynamic_enum_fields_from_schemas;

pub const DEFAULT_STAFF_BATCH_SIZE: usize = 25;
pub const DEFAULT_STAFF_ARCHIVAL_OBJECT_BATCH_SIZE: usize = DEFAULT_STAFF_BATCH_SIZE;

pub const STAFF_INDEXER_RESOLVED_ATTRIBUTES: &[&str] = &[
    "container_locations",
    "container_locations::location_profile",
    "container_profile",
    "location_profile",
    "places",
    "subjects",
    "agent_representation",
    "classifications",
    "digital_object",
    "linked_agents",
    "linked_records",
    "related_agents",
    "repository",
    "repository::agent_representation",
    "top_container",
    "top_container::container_profile",
    "collections",
    "creator",
    "records",
    "related_agents",
    "reviewer",
    "surveyed_by",
    "related_accessions",
    "notes",
];

#[derive(Clone)]
pub struct Client {
    base_url: String,
    http: Arc<HttpClient>,
    pub session_token: Option<String>,
}

#[derive(Deserialize)]
struct LoginResponse {
    session: String,
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http: Arc::new(HttpClient::new()),
            session_token: None,
        }
    }

    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        let url = format!("{}/users/{}/login", self.base_url, username);
        let response = self
            .http
            .post(&url)
            .form(&[("password", password)])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(ASpaceError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let login_res: LoginResponse = response.json().await?;
        self.session_token = Some(login_res.session);
        Ok(())
    }

    pub fn get(&self, path: &str) -> Result<reqwest::RequestBuilder> {
        let token = self
            .session_token
            .as_ref()
            .ok_or(ASpaceError::NotAuthenticated)?;
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        Ok(self.http.get(&url).header("X-ArchivesSpace-Session", token))
    }

    pub async fn repositories(&self) -> Result<Vec<Repository>> {
        let res = self.get("repositories")?.send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn resource_ids(&self, repo_id: u32) -> Result<Vec<u32>> {
        let path = format!("repositories/{}/resources?all_ids=true", repo_id);
        let res = self.get(&path)?.send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn resources_by_ids(&self, repo_id: u32, ids: &[u32]) -> Result<Vec<Resource>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let id_str = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let path = format!("repositories/{}/resources?id_set={}", repo_id, id_str);

        let res = self.get(&path)?.send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn archival_object_ids(&self, repo_id: u32) -> Result<Vec<u32>> {
        self.archival_object_ids_modified_since(repo_id, None).await
    }

    pub async fn archival_object_ids_modified_since(
        &self,
        repo_id: u32,
        modified_since: Option<u64>,
    ) -> Result<Vec<u32>> {
        let path = format!("repositories/{}/archival_objects", repo_id);
        let modified_since_str = modified_since.map(|ts| ts.to_string());
        let mut query: Vec<(&str, &str)> = vec![("all_ids", "true")];
        if let Some(ref ts_str) = modified_since_str {
            query.push(("modified_since", ts_str.as_str()));
        }

        let res = self.get(&path)?.query(&query).send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn archival_objects_by_ids(
        &self,
        repo_id: u32,
        ids: &[u32],
    ) -> Result<Vec<ArchivalObject>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let id_str = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let path = format!(
            "repositories/{}/archival_objects?id_set={}",
            repo_id, id_str
        );

        let res = self.get(&path)?.send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn archival_objects_by_ids_with_top_containers(
        &self,
        repo_id: u32,
        ids: &[u32],
    ) -> Result<Vec<ArchivalObject>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let id_str = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let path = format!("repositories/{}/archival_objects", repo_id);

        let res = self
            .get(&path)?
            .query(&[("id_set", id_str.as_str()), ("resolve[]", "top_container")])
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn digital_object_ids(&self, repo_id: u32) -> Result<Vec<u32>> {
        let path = format!("repositories/{}/digital_objects?all_ids=true", repo_id);
        let res = self.get(&path)?.send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn digital_objects_by_ids(
        &self,
        repo_id: u32,
        ids: &[u32],
    ) -> Result<Vec<crate::models::DigitalObject>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let id_str = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let path = format!("repositories/{}/digital_objects?id_set={}", repo_id, id_str);

        let res = self.get(&path)?.send().await?;
        if !res.status().is_success() {
            return Err(ASpaceError::Api {
                status: res.status().as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }
        Ok(res.json().await?)
    }

    pub async fn fetch_staff_archival_objects_by_ids(
        &self,
        repo_id: u32,
        ids: &[u32],
    ) -> Result<Vec<serde_json::Value>> {
        self.fetch_staff_archival_objects_by_ids_bounded(
            repo_id,
            ids,
            DEFAULT_STAFF_ARCHIVAL_OBJECT_BATCH_SIZE,
        )
        .await
    }

    pub async fn staff_archival_objects_by_ids(
        &self,
        repo_id: u32,
        ids: &[u32],
    ) -> Result<Vec<serde_json::Value>> {
        self.fetch_staff_archival_objects_by_ids(repo_id, ids).await
    }

    pub async fn fetch_staff_archival_objects_by_ids_bounded(
        &self,
        repo_id: u32,
        ids: &[u32],
        max_batch_size: usize,
    ) -> Result<Vec<serde_json::Value>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        if ids.len() > max_batch_size {
            return Err(ASpaceError::BatchSizeExceeded {
                requested: ids.len(),
                max: max_batch_size,
            });
        }

        let mut seen_input_ids = HashSet::with_capacity(ids.len());
        for &id in ids {
            if !seen_input_ids.insert(id) {
                return Err(ASpaceError::InvalidRequest(format!(
                    "Duplicate id {} in request id_set",
                    id
                )));
            }
        }

        let id_str = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let path = format!("repositories/{}/archival_objects", repo_id);

        let mut query_params: Vec<(&str, &str)> =
            Vec::with_capacity(1 + STAFF_INDEXER_RESOLVED_ATTRIBUTES.len());
        query_params.push(("id_set", &id_str));
        for attr in STAFF_INDEXER_RESOLVED_ATTRIBUTES {
            query_params.push(("resolve[]", attr));
        }

        let res = self.get(&path)?.query(&query_params).send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(ASpaceError::Api {
                status: status.as_u16(),
                message: res.text().await.unwrap_or_default(),
            });
        }

        let body = res.text().await?;
        let raw_value: serde_json::Value = serde_json::from_str(&body)?;
        let records = match raw_value {
            serde_json::Value::Array(items) => items,
            _ => {
                return Err(ASpaceError::HydrationValidation(
                    "Response from backend is not a JSON array".to_string(),
                ));
            }
        };

        let expected_prefix = format!("/repositories/{}/archival_objects/", repo_id);
        let mut seen_returned_ids = HashSet::with_capacity(ids.len());

        for (idx, record) in records.iter().enumerate() {
            let obj = record.as_object().ok_or_else(|| {
                ASpaceError::HydrationValidation(format!(
                    "Record at index {} is not a JSON object",
                    idx
                ))
            })?;

            let uri = obj.get("uri").and_then(|v| v.as_str()).ok_or_else(|| {
                ASpaceError::HydrationValidation(format!(
                    "Record at index {} is missing string 'uri' field",
                    idx
                ))
            })?;

            if !uri.starts_with(&expected_prefix) {
                return Err(ASpaceError::HydrationValidation(format!(
                    "Record at index {} has unexpected URI '{}', expected prefix '{}'",
                    idx, uri, expected_prefix
                )));
            }

            let id_part = &uri[expected_prefix.len()..];
            let record_id: u32 = id_part.parse().map_err(|_| {
                ASpaceError::HydrationValidation(format!(
                    "Record at index {} has non-numeric ID in URI '{}'",
                    idx, uri
                ))
            })?;

            if !seen_input_ids.contains(&record_id) {
                return Err(ASpaceError::HydrationValidation(format!(
                    "Record at index {} with ID {} was not in requested id_set",
                    idx, record_id
                )));
            }

            if !seen_returned_ids.insert(record_id) {
                return Err(ASpaceError::HydrationValidation(format!(
                    "Duplicate record returned for ID {} (URI '{}')",
                    record_id, uri
                )));
            }
        }

        if seen_returned_ids.len() != ids.len() {
            let missing: Vec<u32> = ids
                .iter()
                .copied()
                .filter(|id| !seen_returned_ids.contains(id))
                .collect();
            return Err(ASpaceError::HydrationValidation(format!(
                "Missing records for requested IDs: {:?}",
                missing
            )));
        }

        Ok(records)
    }

    /// Fetches all schemas from ArchivesSpace backend `GET /schemas` and derives dynamic enum field
    /// names via breadth-first traversal matching Ruby `IndexerCommon#enum_fields`.
    ///
    /// Requires an authenticated session (`X-ArchivesSpace-Session` header).
    /// Returns the deduplicated field names in first-seen schema order.
    pub async fn dynamic_enum_fields_from_schemas(&self) -> Result<Vec<String>> {
        let res = self.get("schemas")?.send().await?;
        let status = res.status();
        if !status.is_success() {
            return Err(ASpaceError::Api {
                status: status.as_u16(),
                message: format!("HTTP status {}", status),
            });
        }
        let schemas_val: serde_json::Value = res.json().await?;
        extract_dynamic_enum_fields_from_schemas(&schemas_val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn test_url_construction_and_login() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"session": "test-session-123"}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        let res = client.login("admin", "secret").await;

        assert!(res.is_ok());
        assert_eq!(client.session_token.unwrap(), "test-session-123");
    }

    #[tokio::test]
    async fn test_unauthenticated_request_behavior() {
        let client = Client::new("http://dummy");
        let err = client.repositories().await.unwrap_err();
        match err {
            ASpaceError::NotAuthenticated => (),
            _ => panic!("Expected NotAuthenticated error"),
        }
    }

    #[tokio::test]
    async fn test_api_error_mapping() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/users/baduser/login")
            .with_status(403)
            .with_body("Invalid credentials")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        let err = client.login("baduser", "wrong").await.unwrap_err();

        match err {
            ASpaceError::Api { status, message } => {
                assert_eq!(status, 403);
                assert_eq!(message, "Invalid credentials");
            }
            _ => panic!("Expected Api error"),
        }
    }

    #[tokio::test]
    async fn test_empty_resource_list() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _empty_ids = server
            .mock("GET", "/repositories/2/resources?all_ids=true")
            .with_status(200)
            .with_body("[]")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let ids = client.resource_ids(2).await.unwrap();
        assert!(ids.is_empty());

        let resources = client.resources_by_ids(2, &[]).await.unwrap();
        assert!(resources.is_empty());
    }

    #[tokio::test]
    async fn test_resources_by_ids_success() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _resources = server
            .mock("GET", "/repositories/2/resources?id_set=123,456")
            .with_status(200)
            .with_body(
                r#"[
                {
                    "uri": "/repositories/2/resources/123",
                    "title": "Resource 123",
                    "publish": true,
                    "dates": [{"expression": "1990-2000"}],
                    "extents": []
                },
                {
                    "uri": "/repositories/2/resources/456",
                    "title": "Resource 456",
                    "publish": false,
                    "dates": [],
                    "extents": []
                }
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let resources = client.resources_by_ids(2, &[123, 456]).await.unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].uri, "/repositories/2/resources/123");
        assert_eq!(resources[0].title.as_deref(), Some("Resource 123"));
        assert!(resources[0].publish);
    }

    #[tokio::test]
    async fn test_archival_objects_by_ids_success() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _objects = server
            .mock("GET", "/repositories/2/archival_objects?id_set=789")
            .with_status(200)
            .with_body(
                r#"[
                {
                    "uri": "/repositories/2/archival_objects/789",
                    "title": "Object 789",
                    "display_string": "Object 789",
                    "publish": true,
                    "dates": [{"begin": "1995"}],
                    "resource": {
                        "ref": "/repositories/2/resources/123"
                    }
                }
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let objects = client.archival_objects_by_ids(2, &[789]).await.unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].uri, "/repositories/2/archival_objects/789");
        assert_eq!(objects[0].title.as_deref(), Some("Object 789"));
        assert_eq!(
            objects[0].resource.as_ref().unwrap().ref_uri,
            "/repositories/2/resources/123"
        );
    }

    #[tokio::test]
    async fn test_archival_objects_by_ids_with_top_containers_requests_resolution() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let objects_mock = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("id_set".into(), "789".into()),
                mockito::Matcher::UrlEncoded("resolve[]".into(), "top_container".into()),
            ]))
            .with_status(200)
            .with_body(
                r#"[{"uri":"/repositories/2/archival_objects/789","title":"Object 789","instances":[{"instance_type":"text","sub_container":{"top_container":{"ref":"/repositories/2/top_containers/1","_resolved":{"type":"Box","indicator":"1"}}}}]}]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();
        let objects = client
            .archival_objects_by_ids_with_top_containers(2, &[789])
            .await
            .unwrap();

        objects_mock.assert_async().await;
        let instances = objects[0].extra["instances"].as_array().unwrap();
        assert_eq!(
            instances[0]["sub_container"]["top_container"]["_resolved"]["indicator"],
            "1"
        );
    }

    #[tokio::test]
    async fn test_staff_archival_objects_exact_query_auth_and_nested_json() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "test-session-staff-token"}"#)
            .create_async()
            .await;

        let expected_query = format!(
            "id_set=101%2C102&{}",
            STAFF_INDEXER_RESOLVED_ATTRIBUTES
                .iter()
                .map(|attr| format!("resolve%5B%5D={}", attr.replace("::", "%3A%3A")))
                .collect::<Vec<_>>()
                .join("&")
        );

        let response_body = r#"[
            {
                "uri": "/repositories/2/archival_objects/101",
                "title": "AO 101",
                "zebra_field": 1,
                "alpha_field": 2,
                "nested_record": {
                    "resolved_subjects": [
                        {"id": 1, "term": "History", "meta": {"score": 9.5}},
                        {"id": 2, "term": "Archives"}
                    ],
                    "details": {
                        "level": "item",
                        "counts": [10, 20, 30]
                    }
                },
                "middle_field": 3
            },
            {
                "uri": "/repositories/2/archival_objects/102",
                "title": "AO 102",
                "dates": [{"expression": "1950", "date_type": "single"}]
            }
        ]"#;

        let staff_mock = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_header("X-ArchivesSpace-Session", "test-session-staff-token")
            .match_query(mockito::Matcher::Exact(expected_query))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let records = client
            .fetch_staff_archival_objects_by_ids(2, &[101, 102])
            .await
            .unwrap();

        staff_mock.assert_async().await;
        assert_eq!(records.len(), 2);

        // Verify full nested JSON retained
        assert_eq!(records[0]["uri"], "/repositories/2/archival_objects/101");
        assert_eq!(records[0]["title"], "AO 101");
        assert_eq!(
            records[0]["nested_record"]["resolved_subjects"][0]["meta"]["score"],
            9.5
        );
        assert_eq!(records[0]["nested_record"]["details"]["counts"][2], 30);

        // Verify serialized field order is preserved (zebra_field before alpha_field before middle_field)
        let keys: Vec<&str> = records[0]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "uri",
                "title",
                "zebra_field",
                "alpha_field",
                "nested_record",
                "middle_field"
            ]
        );

        // Also test the alias method
        let records_alias = client
            .staff_archival_objects_by_ids(2, &[101, 102])
            .await
            .unwrap();
        assert_eq!(records_alias.len(), 2);
    }

    #[tokio::test]
    async fn test_staff_archival_objects_empty_ids_returns_empty_without_network() {
        let client = Client::new("http://127.0.0.1:9"); // dummy unreachable host, no login
        let res = client
            .fetch_staff_archival_objects_by_ids(2, &[])
            .await
            .unwrap();
        assert!(res.is_empty());

        let res_bounded = client
            .fetch_staff_archival_objects_by_ids_bounded(2, &[], 25)
            .await
            .unwrap();
        assert!(res_bounded.is_empty());
    }

    #[tokio::test]
    async fn test_staff_archival_objects_input_duplicate_rejection() {
        let client = Client::new("http://127.0.0.1:9"); // dummy host, no network needed
        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101, 102, 101])
            .await
            .unwrap_err();

        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains("Duplicate id 101"));
            }
            _ => panic!(
                "Expected InvalidRequest for duplicate input IDs, got: {:?}",
                err
            ),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_batch_bound() {
        let client = Client::new("http://127.0.0.1:9");

        // Default bound is 25: 26 IDs must fail
        let oversized_ids: Vec<u32> = (1..=26).collect();
        let err = client
            .fetch_staff_archival_objects_by_ids(2, &oversized_ids)
            .await
            .unwrap_err();

        match err {
            ASpaceError::BatchSizeExceeded { requested, max } => {
                assert_eq!(requested, 26);
                assert_eq!(max, 25);
            }
            _ => panic!("Expected BatchSizeExceeded, got: {:?}", err),
        }

        // Custom bound with _bounded
        let err_bounded = client
            .fetch_staff_archival_objects_by_ids_bounded(2, &[1, 2, 3], 2)
            .await
            .unwrap_err();

        match err_bounded {
            ASpaceError::BatchSizeExceeded { requested, max } => {
                assert_eq!(requested, 3);
                assert_eq!(max, 2);
            }
            _ => panic!("Expected BatchSizeExceeded, got: {:?}", err_bounded),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_http_errors_mapped() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _err_mock = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(404)
            .with_body("Repository not found")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err {
            ASpaceError::Api { status, message } => {
                assert_eq!(status, 404);
                assert_eq!(message, "Repository not found");
            }
            _ => panic!("Expected Api error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_missing_uri_rejected() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _mock = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"[{"title": "No URI provided"}]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("missing string 'uri'"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_unknown_uri_repo_or_type_rejected() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        // Wrong repo ID in URI
        let _wrong_repo = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"[{"uri": "/repositories/99/archival_objects/101"}]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("unexpected URI"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err),
        }

        // Wrong type (resources instead of archival_objects)
        let mut server2 = Server::new_async().await;
        let _login2 = server2
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _wrong_type = server2
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"[{"uri": "/repositories/2/resources/101"}]"#)
            .create_async()
            .await;

        let mut client2 = Client::new(server2.url());
        client2.login("admin", "pass").await.unwrap();

        let err2 = client2
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err2 {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("unexpected URI"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err2),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_unexpected_and_duplicate_uri_rejected() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        // Backend returns ID 999 when 101 was requested
        let _unexpected_id = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"[{"uri": "/repositories/2/archival_objects/999"}]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("was not in requested id_set"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err),
        }

        // Backend returns duplicate record for same ID
        let mut server2 = Server::new_async().await;
        let _login2 = server2
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _dup_record = server2
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/101"},
                {"uri": "/repositories/2/archival_objects/101"}
            ]"#,
            )
            .create_async()
            .await;

        let mut client2 = Client::new(server2.url());
        client2.login("admin", "pass").await.unwrap();

        let err2 = client2
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err2 {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("Duplicate record returned for ID 101"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err2),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_missing_records_rejected() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        // Requested 101 and 102, but only 101 returned
        let _missing = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"[{"uri": "/repositories/2/archival_objects/101"}]"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101, 102])
            .await
            .unwrap_err();

        match err {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("Missing records for requested IDs: [102]"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_staff_archival_objects_preserves_backend_response_order() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        // Backend returns records in reverse of requested order: [103, 101, 102]
        let _order = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(
                r#"[
                {"uri": "/repositories/2/archival_objects/103", "title": "Three"},
                {"uri": "/repositories/2/archival_objects/101", "title": "One"},
                {"uri": "/repositories/2/archival_objects/102", "title": "Two"}
            ]"#,
            )
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let records = client
            .fetch_staff_archival_objects_by_ids(2, &[101, 102, 103])
            .await
            .unwrap();

        // Must preserve the backend's response order [103, 101, 102], not reorder
        assert_eq!(records[0]["uri"], "/repositories/2/archival_objects/103");
        assert_eq!(records[1]["uri"], "/repositories/2/archival_objects/101");
        assert_eq!(records[2]["uri"], "/repositories/2/archival_objects/102");
    }

    #[tokio::test]
    async fn test_staff_archival_objects_not_json_array_rejected() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _not_array = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"error": "unexpected format"}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .fetch_staff_archival_objects_by_ids(2, &[101])
            .await
            .unwrap_err();

        match err {
            ASpaceError::HydrationValidation(msg) => {
                assert!(msg.contains("not a JSON array"));
            }
            _ => panic!("Expected HydrationValidation, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_archival_object_ids_modified_since_none() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let mock_none = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_header("X-ArchivesSpace-Session", "token")
            .match_query(mockito::Matcher::Exact("all_ids=true".to_string()))
            .with_status(200)
            .with_body("[101, 102]")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let ids = client
            .archival_object_ids_modified_since(2, None)
            .await
            .unwrap();
        assert_eq!(ids, vec![101, 102]);
        mock_none.assert_async().await;
    }

    #[tokio::test]
    async fn test_archival_object_ids_backward_compatibility() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let mock_legacy = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_header("X-ArchivesSpace-Session", "token")
            .match_query(mockito::Matcher::Exact("all_ids=true".to_string()))
            .with_status(200)
            .with_body("[101, 102]")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let ids = client.archival_object_ids(2).await.unwrap();
        assert_eq!(ids, vec![101, 102]);
        mock_legacy.assert_async().await;
    }

    #[tokio::test]
    async fn test_archival_object_ids_modified_since_zero() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let mock_zero = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_header("X-ArchivesSpace-Session", "token")
            .match_query(mockito::Matcher::Exact(
                "all_ids=true&modified_since=0".to_string(),
            ))
            .with_status(200)
            .with_body("[201, 202, 203]")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let ids = client
            .archival_object_ids_modified_since(2, Some(0))
            .await
            .unwrap();
        assert_eq!(ids, vec![201, 202, 203]);
        mock_zero.assert_async().await;
    }

    #[tokio::test]
    async fn test_archival_object_ids_modified_since_timestamp() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let mock_timestamp = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_header("X-ArchivesSpace-Session", "token")
            .match_query(mockito::Matcher::Exact(
                "all_ids=true&modified_since=1700000000".to_string(),
            ))
            .with_status(200)
            .with_body("[301]")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let ids = client
            .archival_object_ids_modified_since(2, Some(1700000000))
            .await
            .unwrap();
        assert_eq!(ids, vec![301]);
        mock_timestamp.assert_async().await;
    }

    #[tokio::test]
    async fn test_archival_object_ids_modified_since_http_error() {
        let mut server = Server::new_async().await;

        let _login = server
            .mock("POST", "/users/admin/login")
            .with_status(200)
            .with_body(r#"{"session": "token"}"#)
            .create_async()
            .await;

        let _err_mock = server
            .mock("GET", "/repositories/2/archival_objects")
            .match_query(mockito::Matcher::Exact(
                "all_ids=true&modified_since=0".to_string(),
            ))
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.login("admin", "pass").await.unwrap();

        let err = client
            .archival_object_ids_modified_since(2, Some(0))
            .await
            .unwrap_err();

        match err {
            ASpaceError::Api { status, message } => {
                assert_eq!(status, 500);
                assert_eq!(message, "Internal Server Error");
            }
            _ => panic!("Expected Api error, got: {:?}", err),
        }

        let unauthed_client = Client::new("http://dummy");
        let err_unauthed = unauthed_client
            .archival_object_ids_modified_since(2, Some(0))
            .await
            .unwrap_err();
        match err_unauthed {
            ASpaceError::NotAuthenticated => (),
            _ => panic!("Expected NotAuthenticated error, got: {:?}", err_unauthed),
        }
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_unauthenticated_short_circuit() {
        let client = Client::new("http://dummy");
        let err = client.dynamic_enum_fields_from_schemas().await.unwrap_err();
        match err {
            ASpaceError::NotAuthenticated => (),
            _ => panic!("Expected NotAuthenticated error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_http_error() {
        let mut server = Server::new_async().await;
        let sentinel_secret = "SENTINEL_PRIVATE_BACKEND_SCHEMA_SECRET_DO_NOT_LEAK_XYZ789";

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(500)
            .with_body(sentinel_secret)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let err = client.dynamic_enum_fields_from_schemas().await.unwrap_err();
        match &err {
            ASpaceError::Api { status, message } => {
                assert_eq!(*status, 500);
                assert_eq!(message, "HTTP status 500 Internal Server Error");
                assert!(!message.contains(sentinel_secret));
            }
            _ => panic!("Expected Api error, got: {:?}", err),
        }

        let display_repr = format!("{}", err);
        let debug_repr = format!("{:?}", err);
        assert!(
            !display_repr.contains(sentinel_secret),
            "Sentinel private content leaked in Display: {}",
            display_repr
        );
        assert!(
            !debug_repr.contains(sentinel_secret),
            "Sentinel private content leaked in Debug: {}",
            debug_repr
        );
        assert!(display_repr.contains("500"));
        assert!(debug_repr.contains("500"));
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_malformed_top_level_shape() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[1, 2, 3]")
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let err = client.dynamic_enum_fields_from_schemas().await.unwrap_err();
        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains("Top-level /schemas response must be a JSON object"));
            }
            _ => panic!("Expected InvalidRequest error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_malformed_model_shape() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"bad_model": "not an object"}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let err = client.dynamic_enum_fields_from_schemas().await.unwrap_err();
        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains("Model schema for 'bad_model' is not a JSON object"));
            }
            _ => panic!("Expected InvalidRequest error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_malformed_properties_shape() {
        let mut server = Server::new_async().await;

        let _mock_string = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model_a": {"properties": "invalid_string"}}"#)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let err = client.dynamic_enum_fields_from_schemas().await.unwrap_err();
        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains(
                    "Model schema for 'model_a' has malformed 'properties' field (expected object)"
                ));
            }
            _ => panic!("Expected InvalidRequest error, got: {:?}", err),
        }

        // Also test array properties
        let mut server2 = Server::new_async().await;
        let _mock_arr = server2
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model_b": {"properties": [1, 2, 3]}}"#)
            .create_async()
            .await;

        let mut client2 = Client::new(server2.url());
        client2.session_token = Some("token123".to_string());

        let err2 = client2
            .dynamic_enum_fields_from_schemas()
            .await
            .unwrap_err();
        match err2 {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains(
                    "Model schema for 'model_b' has malformed 'properties' field (expected object)"
                ));
            }
            _ => panic!("Expected InvalidRequest error, got: {:?}", err2),
        }
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_malformed_items_shape() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "model_with_bad_items": {
                "properties": {
                    "bad_array": {
                        "type": "array",
                        "items": "malformed_string_not_object"
                    }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let err = client.dynamic_enum_fields_from_schemas().await.unwrap_err();
        match err {
            ASpaceError::InvalidRequest(msg) => {
                assert!(msg.contains(
                    "Property 'bad_array' has malformed 'items' field (expected object)"
                ));
            }
            _ => panic!("Expected InvalidRequest error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_short_circuit_malformed_items() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "model_short_circuit": {
                "properties": {
                    "field_valid_enum_malformed_items": {
                        "type": "string",
                        "dynamic_enum": "valid",
                        "items": "malformed"
                    }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        assert_eq!(fields, vec!["field_valid_enum_malformed_items"]);
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_empty_models() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "empty1": {},
            "empty2": {"type": "object"},
            "empty3": {"properties": {}},
            "empty4": {"properties": null}
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        assert!(fields.is_empty());
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_nested_schemas() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "archival_object": {
                "type": "object",
                "properties": {
                    "level": { "type": "string", "dynamic_enum": "archival_record_level" },
                    "dates": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "date_label": { "type": "string", "dynamic_enum": "date_label" },
                                "date_type": { "type": "string", "dynamic_enum": "date_type" }
                            }
                        }
                    }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        assert_eq!(fields, vec!["level", "date_label", "date_type"]);
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_array_items() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "rights_statement": {
                "properties": {
                    "local_access_restriction_type": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "dynamic_enum": "restriction_type"
                        }
                    }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        assert_eq!(fields, vec!["local_access_restriction_type"]);
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_ruby_truthiness_edge() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "truthiness_model": {
                "properties": {
                    "f_false": { "type": "string", "dynamic_enum": false },
                    "f_null": { "type": "string", "dynamic_enum": null },
                    "f_true": { "type": "string", "dynamic_enum": true },
                    "f_zero": { "type": "string", "dynamic_enum": 0 },
                    "f_empty_str": { "type": "string", "dynamic_enum": "" },
                    "f_str": { "type": "string", "dynamic_enum": "valid_enum" },
                    "f_empty_arr": { "type": "string", "dynamic_enum": [] },
                    "f_empty_obj": { "type": "string", "dynamic_enum": {} },
                    "arr_items_false": { "type": "array", "items": { "dynamic_enum": false } },
                    "arr_items_null": { "type": "array", "items": { "dynamic_enum": null } },
                    "arr_items_truthy": { "type": "array", "items": { "dynamic_enum": "items_enum" } }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        assert_eq!(
            fields,
            vec![
                "f_true",
                "f_zero",
                "f_empty_str",
                "f_str",
                "f_empty_arr",
                "f_empty_obj",
                "arr_items_truthy"
            ]
        );
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_items_exclusion() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "model_with_items": {
                "properties": {
                    "some_array": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "dynamic_enum": "array_enum"
                        }
                    },
                    "items": {
                        "type": "string",
                        "dynamic_enum": "creeps_in"
                    }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        assert_eq!(fields, vec!["some_array"]);
    }

    #[tokio::test]
    async fn test_dynamic_enum_fields_ordering_and_repeated_fields() {
        let mut server = Server::new_async().await;

        let body = r#"{
            "model_z": {
                "properties": {
                    "z_field": { "dynamic_enum": "z_enum" },
                    "a_field": { "dynamic_enum": "a_enum" }
                }
            },
            "model_a": {
                "properties": {
                    "m_field": { "dynamic_enum": "m_enum" },
                    "z_field": { "dynamic_enum": "z_enum" }
                }
            }
        }"#;

        let _mock = server
            .mock("GET", "/schemas")
            .match_header("X-ArchivesSpace-Session", "token123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let mut client = Client::new(server.url());
        client.session_token = Some("token123".to_string());

        let fields = client.dynamic_enum_fields_from_schemas().await.unwrap();
        // First-seen order: "z_field", "a_field", "m_field"
        // NOT sorted alphabetically ("a_field", "m_field", "z_field")
        // Repeated "z_field" deduplicated
        assert_eq!(fields, vec!["z_field", "a_field", "m_field"]);
    }
}
