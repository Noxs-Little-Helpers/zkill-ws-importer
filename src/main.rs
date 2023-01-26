mod models;

extern crate core;

use futures_util::{SinkExt, StreamExt};
use tokio::{
    net::TcpStream,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
};
use tokio_tungstenite::{
    connect_async, MaybeTlsStream, WebSocketStream,
    tungstenite::Message::{Binary, Ping, Pong, Text},
};
use serde_json::{Value};
use std::env;
use std::str::FromStr;
use tracing::{error, info, warn, debug, trace, Level};
use tracing_subscriber::FmtSubscriber;
use models::{
    app_config,
};

#[tokio::main]
async fn main() {
    let app_config: app_config::AppConfig = load_config();
    config_logging(match &app_config.logging {
        None => { Level::INFO }
        Some(logging_config) => {
            match Level::from_str(&logging_config.logging_level.as_str()) {
                Err(e) => {
                    panic!("Invalid logging level supplied in config. Supplied value [{0}] Error [{1:?}]", &logging_config.logging_level, e);
                }
                Ok(level) => { level }
            }
        }
    });
    info!("zkill-ws-importer started");
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
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
#[tracing::instrument]
async fn read_from_ws(sender_channel: UnboundedSender<String>, app_config: app_config::AppConfig) {
    info!("Web Socket: Starting connection");
    loop {
        let ws_stream = match start_websocket(&app_config.websocket.url).await {
            Ok((stream, _)) => {
                info!("Web Socket: Connected");
                stream
            }
            Err(error) => {
                error!("Web Socket: Could not connect. Reattempting... [{:?}]", error);
                continue;
            }
        };
        let (mut write, read) = ws_stream.split();

        info!("Web Socket: Attempting to send subscribe message");
        match write.send(app_config.websocket.sub_message.clone().into()).await {
            Ok(_) => { info!("Web Socket: Subscription message sent successfully") }
            Err(err) => {
                error!("Web Socket: Unable to subscribe. Reattempting... [{:?}]", err);
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
                    error!("Web Socket: Error received on connection {0:?}", error);
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
                debug!("Web Socket: Wrote message to channel [{0}]",print_str);
            } else {
                debug!("Web Socket: Received empty message. Will be ignored");
                // tokio::io::stdout().write_all("Empty message\n".as_bytes()).await.unwrap();
            }
        }).await;
    }
}

#[tracing::instrument]
async fn write_to_database(mut receiver_channel: UnboundedReceiver<String>, app_config: app_config::AppConfig) {
    let client: mongodb::Client = match connect_to_db(&app_config.database.conn_string).await {
        Ok(client) => {
            client
        }
        Err(error) => {
            panic!("Database: Unable to create database client. Dont know how to proceed so panicking [{0:?}]", error);
        }
    };
    let database = client.database(&app_config.database.database_name);
    let collection = database.collection(&app_config.database.killmail_collection);
    {
        info!("Database: Attempting to connect");
        let mut test_ping_successful = false;
        loop {
            match ping_db(&database).await {
                Ok(_) => { test_ping_successful = true }
                Err(error) => {
                    error!("Database: Unable to ping. Reattempting... [{0:?}]", error);
                }
            }
            if test_ping_successful {
                info!("Database: Connection established");
                break;
            }
        }
    }
    while let Some(ws_message) = receiver_channel.recv().await {
        let mut able_to_write_to_database;
        let mut had_disconnect = false;
        loop {
            // let model: zkillboard::ZKillmail = serde_json::from_str(ws_message).unwrap();
            let value: Value = match serde_json::from_str(&ws_message) {
                Ok(value) => { value }
                Err(error) => {
                    trace!("Database: Message could not be parsed by serde_json [{0}] [{1:?}]", ws_message, error);
                    break;//Skip the message it might not be valid json
                }
            };
            {
                let killmail_id = match value.as_object() {
                    None => {
                        continue;
                    }
                    Some(value) => {
                        match value.get("killmail_id") {
                            None => {
                                continue;
                            }
                            Some(killmail_value) => {
                                match killmail_value.as_i64() {
                                    None => {
                                        continue;
                                    }
                                    Some(killmail_string) => {
                                        killmail_string
                                    }
                                }
                            }
                        }
                    }
                };

                if is_in_collection(&killmail_id, &collection).await {
                    info!("Database: Got kill from ws that is already in database [{}]. Skipping...", killmail_id);
                    continue;
                }
            }
            let bson_doc = match mongodb::bson::to_bson(&value) {
                Ok(value) => { value }
                Err(error) => {
                    error!("Database: Json from serde_json could not be parsed by bson [{0}] [{1:?}]", ws_message, error);
                    break;//Skip the message it might not be valid json
                }
            };
            match write_to_db(bson_doc, &collection).await {
                Ok(inserted_id) => {
                    able_to_write_to_database = true;
                    debug!("Database: Document inserted");
                    trace!("Database: Document inserted ID: [{0}]", inserted_id.inserted_id);
                }
                Err(error) => {
                    let error_msg = format!("Database: Got error attempting to write to database message [{0:?}] [{1}]", &error, ws_message);
                    match *error.kind {
                        mongodb::error::ErrorKind::Write(details) => {
                            match details {
                                mongodb::error::WriteFailure::WriteConcernError(_) => {}
                                mongodb::error::WriteFailure::WriteError(write_error) => {
                                    if write_error.code == 11000 {
                                        debug!("Database: Got duplicate key error. Skipping...");
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    error!("{}", &error_msg);
                    had_disconnect = true;
                    continue;//Dont skip the message. We should wait for db to reconnect
                }
            }
            if able_to_write_to_database {
                if had_disconnect { info!("Database: Reconnected"); }
                break;
            };
        }
    }
}

async fn is_in_collection(id: &i64, collection: &mongodb::Collection<mongodb::bson::Bson>) -> bool {
    return match collection.find_one(mongodb::bson::doc! {"killmail_id": id}, None).await {
        Ok(result) => {
            match result {
                None => { false }
                Some(_) => { true }
            }
        }
        Err(_) => { false }
    };
}

async fn write_to_db(write_value: mongodb::bson::Bson, collection: &mongodb::Collection<mongodb::bson::Bson>) -> Result<mongodb::results::InsertOneResult, mongodb::error::Error> {
    return collection.insert_one(write_value, None).await;
}

async fn connect_to_db(connect_addr: &String) -> mongodb::error::Result<mongodb::Client> {
    let client_options = mongodb::options::ClientOptions::parse(connect_addr).await?;
    return mongodb::Client::with_options(client_options);
}

async fn ping_db(database: &mongodb::Database) -> Result<mongodb::bson::Document, mongodb::error::Error> {
    return database
        .run_command(mongodb::bson::doc! {"ping": 1}, None).await;
}

fn load_config() -> app_config::AppConfig {
    let args: Vec<String> = env::args().collect();
    let config_loc = match args.get(1) {
        Some(loc) => {
            loc
        }
        None => {
            panic!("Config file not specified in first argument");
        }
    };

    let imported_config = config::Config::builder()
        .add_source(config::File::with_name(config_loc))
        .add_source(config::Environment::with_prefix("NLH"))
        .build()
        .unwrap();
    return imported_config
        .try_deserialize::<app_config::AppConfig>()
        .unwrap();
}

async fn start_websocket(connect_addr: &String) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, tokio_tungstenite::tungstenite::handshake::client::Response), tokio_tungstenite::tungstenite::Error> {
    let url = match url::Url::parse(&connect_addr) {
        Ok(result) => { result }
        Err(error) => {
            panic!("Invalid websocket url [{0}] Error[{1}]", connect_addr, error);
        }
    };
    return connect_async(url).await;
}

fn config_logging(logging_level: Level) {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(logging_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Setting default logging subscriber failed");
}