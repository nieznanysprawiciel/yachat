use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use ya_service_bus::RpcMessage;

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ChatError {
    #[error("Text Message Rejected")]
    Rejected,
    #[error("Unknown user.")]
    UnknownUser,
    #[error("NodeId is invalid. Wrong format.")]
    InvalidNodeId,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessage {
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendText {
    pub messages: Vec<TextMessage>,
}

impl RpcMessage for SendText {
    const ID: &'static str = "SendText";
    type Item = ();
    type Error = ChatError;
}
