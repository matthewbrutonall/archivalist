use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::{ASpaceError, Result};
use crate::models::{ArchivalObject, Repository, Resource};

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
}
