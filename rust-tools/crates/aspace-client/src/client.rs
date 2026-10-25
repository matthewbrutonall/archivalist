use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;

use crate::error::{ASpaceError, Result};
use crate::models::{ArchivalObject, Repository, Resource};

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
        let path = format!("repositories/{}/archival_objects?all_ids=true", repo_id);
        let res = self.get(&path)?.send().await?;
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
}
