use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
  pub id: String,
  pub sender: String,
  pub recipients: Vec<String>,
  pub subject: Option<String>,
  pub size: i64,
  pub has_attachments: bool,
  pub is_read: bool,
  pub is_starred: bool,
  pub tags: Vec<String>,
  pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
  pub id: String,
  pub sender: String,
  pub recipients: Vec<String>,
  pub subject: Option<String>,
  pub text_body: Option<String>,
  pub html_body: Option<String>,
  pub size: i64,
  pub has_attachments: bool,
  pub is_read: bool,
  pub is_starred: bool,
  pub tags: Vec<String>,
  pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
  pub messages: Vec<MessageSummary>,
  pub total: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsEvent {
  #[serde(rename = "message:new")]
  MessageNew(MessageSummary),
  #[serde(rename = "message:delete")]
  MessageDelete { id: String },
  #[serde(rename = "message:read")]
  MessageRead { id: String, is_read: bool },
  #[serde(rename = "message:starred")]
  MessageStarred { id: String, is_starred: bool },
  #[serde(rename = "message:tags")]
  MessageTags { id: String, tags: Vec<String> },
  #[serde(rename = "messages:clear")]
  MessagesClear,
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ApiClient {
  client: reqwest::Client,
  base_url: String,
}

impl ApiClient {
  pub fn new(base_url: String) -> Self {
    let client = reqwest::Client::builder()
      .timeout(REQUEST_TIMEOUT)
      .connect_timeout(Duration::from_secs(5))
      .build()
      .expect("failed to build HTTP client");

    Self { client, base_url }
  }

  pub async fn list_messages(
    &self,
    query: Option<&str>,
    limit: i64,
    offset: i64,
  ) -> Result<ListResponse> {
    let url = format!("{}/api/v1/messages", self.base_url);
    let mut req = self
      .client
      .get(&url)
      .query(&[("limit", limit.to_string()), ("offset", offset.to_string())]);

    if let Some(q) = query {
      req = req.query(&[("q", q)]);
    }

    let resp = req.send().await?.error_for_status()?.json().await?;
    Ok(resp)
  }

  pub async fn get_message(&self, id: &str) -> Result<Message> {
    let url = format!("{}/api/v1/messages/{}", self.base_url, id);
    let resp = self.client.get(&url).send().await?.error_for_status()?.json().await?;
    Ok(resp)
  }

  pub async fn delete_message(&self, id: &str) -> Result<()> {
    let url = format!("{}/api/v1/messages/{}", self.base_url, id);
    self.client.delete(&url).send().await?.error_for_status()?;
    Ok(())
  }

  pub async fn delete_all_messages(&self) -> Result<()> {
    let url = format!("{}/api/v1/messages", self.base_url);
    self.client.delete(&url).send().await?.error_for_status()?;
    Ok(())
  }

  pub async fn update_message(
    &self,
    id: &str,
    is_read: Option<bool>,
    is_starred: Option<bool>,
  ) -> Result<()> {
    let url = format!("{}/api/v1/messages/{}", self.base_url, id);
    let mut body = serde_json::Map::new();
    if let Some(v) = is_read {
      body.insert("is_read".into(), serde_json::Value::Bool(v));
    }
    if let Some(v) = is_starred {
      body.insert("is_starred".into(), serde_json::Value::Bool(v));
    }
    self.client.patch(&url).json(&body).send().await?.error_for_status()?;
    Ok(())
  }

  pub async fn get_raw_message(&self, id: &str) -> Result<String> {
    let url = format!("{}/api/v1/messages/{}/raw", self.base_url, id);
    let resp = self.client.get(&url).send().await?.error_for_status()?.text().await?;
    Ok(resp)
  }
}
