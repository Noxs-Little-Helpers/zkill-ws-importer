use serde::{Deserialize, Serialize};

#[derive(Debug)]
#[derive(Serialize, Deserialize)]
#[derive(Clone)]
pub struct AppConfig {
    pub websocket: WebsocketConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug)]
#[derive(Serialize, Deserialize)]
#[derive(Clone)]
pub struct WebsocketConfig {
    pub url: String,
    pub sub_message: String,
    pub client_id: String,
}

#[derive(Debug)]
#[derive(Serialize, Deserialize)]
#[derive(Clone)]
pub struct DatabaseConfig {
    pub conn_string: String,
    pub database_name: String,
    pub collection_name: String,
}

#[derive(Debug)]
#[derive(Serialize, Deserialize)]
#[derive(Clone)]
pub struct LoggingConfig {
    pub logging_dir: String,
}