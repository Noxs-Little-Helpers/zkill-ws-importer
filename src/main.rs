mod database;
mod models;

use crate::models::config::LoggingConfig;

extern crate core;

use futures_util::{future, pin_mut, SinkExt, StreamExt, TryFutureExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tokio_tungstenite::{
    connect_async, MaybeTlsStream, tungstenite::protocol::Message, WebSocketStream,
    tungstenite::Message::{Binary, Ping, Pong, Text},
};
use serde::{Deserialize, Serialize};
use mongodb::{bson, bson::Document, Client, Collection, Database};
use serde_json::Value;
use std::env;
use std::fs;
use std::sync::Arc;
use log::{error, info, warn, LevelFilter, debug};
use log4rs::{
    append::{
        console::{
            ConsoleAppender,
            Target,
        },
        file::FileAppender,
        rolling_file::{
            policy::{
                compound::CompoundPolicy,
                compound::roll::fixed_window::FixedWindowRoller,
                compound::trigger::size::SizeTrigger,
            },
            RollingFileAppender,
        },
    },
    encode::{pattern::PatternEncoder, json::JsonEncoder},
    config::{Appender, Config, Logger, Root},
    filter::threshold::ThresholdFilter,
};
use mongodb::{
    bson::{Bson, doc},
    options::ClientOptions,
    results::InsertOneResult,
};
use std::sync::Mutex;


use models::{
    zkillboard,
    config,
    config::AppConfig,
};
use tungstenite::{
    error::{UrlError},
    handshake::client::Response,
    protocol::WebSocketConfig,
};

#[tokio::main]
async fn main() {
    let app_config: config::AppConfig = load_config();
    config_logging(&app_config.logging);
    info!("zkill-ws-importer started");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    // let app_config_arc = Arc::new(app_config);
    let clone1 = app_config.clone();
    let clone2 = app_config.clone();
    let join_ws = tokio::spawn(async move {
        read_from_ws(tx, clone1).await;
    });
    let join_db = tokio::spawn(async move {
        write_to_database(rx, clone2).await
    });
    futures_util::future::join_all(vec![join_ws, join_db]).await;
}

// async fn read_from_ws_to_cache(cache_list: Arc<Mutex<Vec<&String>>>, app_config: Arc<AppConfig>) {
async fn read_from_ws(sender_channel: UnboundedSender<String>, app_config: AppConfig) {
    info!("Web Socket: Starting connection");
    loop {
        let ws_stream = match start_websocket(&app_config.websocket.url).await {
            Ok((stream, _)) => {
                info!("Web Socket: Connected");
                stream
            }
            Err(error) => {
                error!("Web Socket: Could not connect. Reattempting...");
                continue;
            }
        };
        let (mut write, read) = ws_stream.split();

        info!("Web Socket: Attempting to send subscribe message");
        match write.send(app_config.websocket.sub_message.clone().into()).await {
            Ok(_) => { info!("Web Socket: Subscription message sent successfully") }
            Err(err) => {
                error!("Web Socket: Unable to subscribe. Reattempting...");
                continue;
            }
        }

        read.for_each(|message| async {
            // let ws_message_text = message.unwrap().into_text().unwrap();
            let ws_message_text = match message {
                Ok(message) => match message {
                    Text(text) => text,
                    Binary(binary) | Ping(binary) | Pong(binary) => {
                        // panic!("{}", String::from_utf8(binary).unwrap())
                        match String::from_utf8(binary) {
                            Ok(value) => { value }
                            Err(error) => {
                                //TODO enhance error
                                warn!("Web Socket: Got non string message {0}", error);
                                return;
                            }
                        }
                    }
                    unhandled_type => {
                        warn!("Web Socket: Got non string message {0}", unhandled_type);
                        return;
                    }
                }
                Err(error) => {
                    warn!("Web Socket: Error received on connection {0:?}", error);
                    return;
                }
            };
            if !ws_message_text.is_empty() {
                let print_str = "[".to_owned() + &ws_message_text.clone() + "]\n";
                // tokio::io::stdout().write_all(print_str.as_bytes()).await.unwrap();
                match sender_channel.send(ws_message_text) {
                    Ok(_) => {}
                    Err(error) => {
                        error!("Web Socket: Cannot send message on channel [{0}]", error);
                        panic!("Web Socket: Cannot send message on channel [{0}]", error);
                    }
                };
                info!("Web Socket: Wrote message to channel");
                debug!("Web Socket: Wrote message to channel [{0}]",print_str);
            } else {
                info!("Web Socket: Received empty message");
                debug!("Web Socket: Received empty message. Will be ignored");
                // tokio::io::stdout().write_all("Empty message\n".as_bytes()).await.unwrap();
            }
        }).await;
    }
}

async fn write_to_database(mut receiver_channel: UnboundedReceiver<String>, app_config: AppConfig) {
    let client: Client = match connect_to_db(&app_config.database.conn_string).await {
        Ok(client) => {
            client
        }
        Err(error) => {
            error!("Database: Unable to create database client");
            panic!("Database: Unable to create database client. Dont know how to proceed so panicking");
        }
    };
    let database = client.database(&app_config.database.database_name);
    let collection = database.collection(&app_config.database.collection_name);
    {
        info!("Database: Attempting to connect");
        let mut test_ping_successful = false;
        loop {
            match ping_db(&database).await {
                Ok(document) => { test_ping_successful = true }
                Err(error) => {
                    error!("Database: Unable to ping. Reattempting... [{0:?}]", error);
                }
            }
            if (test_ping_successful) {
                info!("Database: Connection established");
                break;
            }
        }
    }
    while let Some(ws_message) = receiver_channel.recv().await {
        let mut able_to_write_to_database = false;
        let mut had_disconnect = false;
        loop {
            // let model: zkillboard::ZKillmail = serde_json::from_str(ws_message).unwrap();
            let value: Value = match serde_json::from_str(&ws_message) {
                Ok(value) => { value }
                Err(error) => {
                    warn!("Database: Message could not be parsed by serde_json [{0}] [{1:?}]", ws_message, error);
                    break;//Skip the message it might not be valid json
                }
            };
            let bson_doc = match bson::to_bson(&value) {
                Ok(value) => { value }
                Err(error) => {
                    warn!("Database: Json from serde_json could not be parsed by bson [{0}] [{1:?}]", ws_message, error);
                    break;//Skip the message it might not be valid json
                }
            };
            match write_to_db(bson_doc, &collection).await {
                Ok(inserted_id) => {
                    able_to_write_to_database = true;
                    info!("Database: Document inserted");
                    debug!("Database: Document inserted ID: [{0}]", inserted_id.inserted_id);
                }
                Err(error) => {
                    error!("Database: Got error attempting to write to database message[{0}] [{1:?}]", ws_message, error);
                    had_disconnect = true;
                    continue;//Dont skip the message. We should wait for db to reconnect
                }
            }
            if (able_to_write_to_database) {
                if had_disconnect { info!("Database: Reconnected"); }
                break;
            };
        }
    }
}


async fn write_to_db(write_value: Bson, collection: &Collection<Bson>) -> Result<InsertOneResult, mongodb::error::Error> {
    return collection.insert_one(write_value, None).await;
}

async fn connect_to_db(connect_addr: &String) -> mongodb::error::Result<Client> {
    let mut client_options = ClientOptions::parse(connect_addr).await?;
    return Client::with_options(client_options);
}

async fn ping_db(database: &Database) -> Result<Document, mongodb::error::Error> {
    return database
        .run_command(doc! {"ping": 1}, None).await;
}

fn load_config() -> config::AppConfig {
    let args: Vec<String> = env::args().collect();
    let config_loc = match args.get(1) {
        Some(loc) => {
            loc
        }
        None => {
            panic!("Config file not specified in first argument");
        }
    };
    let contents = fs::read_to_string(config_loc)
        .expect(&format!("Cannot open config file {0}", config_loc));

    let model: config::AppConfig = serde_json::from_str(&contents).unwrap();
    // println!("{:?}", &model);
    return model;
}

async fn start_websocket(connect_addr: &String) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, tokio_tungstenite::tungstenite::handshake::client::Response), tokio_tungstenite::tungstenite::Error> {
    let url = match url::Url::parse(&connect_addr) {
        Ok(result) => { result }
        Err(error) => {
            error!("Invalid websocket url [{0}] Error[{1}]", connect_addr, error);
            panic!("Invalid websocket url [{0}] Error[{1}]", connect_addr, error);
        }
    };
    return connect_async(url).await;
}

fn config_logging(logging_config: &LoggingConfig) {
    let mut active_log = String::new();
    {
        active_log.push_str(&logging_config.dir);
        active_log.push_str("/");
        active_log.push_str(&logging_config.active_file);
    }
    let mut archive_patter = String::new();
    {
        archive_patter.push_str(&logging_config.dir);
        archive_patter.push_str("/");
        archive_patter.push_str(&logging_config.archive_pattern);
    }
    let log_to_stdout = ConsoleAppender::builder().target(Target::Stdout)
        .build();
    // Build a file logger.
    let log_to_file = RollingFileAppender::builder()
        .encoder(Box::new(JsonEncoder::new()))
        .build(&active_log,
               Box::new(CompoundPolicy::new(
                   Box::new(SizeTrigger::new(10 * 1024 * 1024)),
                   Box::new(FixedWindowRoller::builder().build(&archive_patter, 10).unwrap()),
               )))
        .unwrap();

    // Log Debug level output to file where debug is the default level
    // and the programmatically specified level to stderr.
    let config = Config::builder()
        .appender(Appender::builder().build("log_to_file", Box::new(log_to_file)))
        .appender(
            Appender::builder()
                .filter(Box::new(ThresholdFilter::new(log::LevelFilter::Info)))
                .build("log_to_stdout", Box::new(log_to_stdout)),
        )
        .build(
            Root::builder()
                .appender("log_to_file")
                .appender("log_to_stdout")
                .build(LevelFilter::Debug),
        )
        .unwrap();

    log4rs::init_config(config).unwrap();
}