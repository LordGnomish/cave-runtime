//! MongoDB command dispatcher.

pub mod hello;
pub mod db;
pub mod crud;
pub mod agg;
pub mod index;
pub mod cursor;
pub mod admin;

use crate::bson::Document;
use crate::engine::Engine;
use crate::cursor::CursorStore;
use serde_json::Value;
use std::sync::Arc;

pub async fn dispatch(
    cmd_name: &str,
    cmd_doc: &Document,
    engine: Arc<Engine>,
    cursors: Arc<CursorStore>,
) -> Result<Document, String> {
    match cmd_name {
        "isMaster" | "hello" | "ismaster" => hello::hello(cmd_doc).await,
        "ping" => hello::ping().await,
        "buildInfo" => hello::build_info().await,
        "listDatabases" => db::list_databases(engine).await,
        "listCollections" => db::list_collections(cmd_doc, engine).await,
        "createCollection" => db::create_collection(cmd_doc, engine).await,
        "drop" => db::drop_collection(cmd_doc, engine).await,
        "dropDatabase" => db::drop_database(cmd_doc, engine).await,
        "find" => crud::find(cmd_doc, engine, cursors).await,
        "insert" | "insertOne" | "insertMany" => crud::insert(cmd_doc, engine).await,
        "update" | "updateOne" | "updateMany" => crud::update(cmd_doc, engine).await,
        "delete" | "deleteOne" | "deleteMany" => crud::delete(cmd_doc, engine).await,
        "count" | "countDocuments" => crud::count(cmd_doc, engine).await,
        "aggregate" => agg::aggregate(cmd_doc, engine).await,
        "createIndexes" => index::create_indexes(cmd_doc, engine).await,
        "dropIndexes" | "dropIndex" => index::drop_indexes(cmd_doc, engine).await,
        "listIndexes" => index::list_indexes(cmd_doc, engine).await,
        "getMore" => cursor::get_more(cmd_doc, cursors).await,
        "killCursors" => cursor::kill_cursors(cmd_doc, cursors).await,
        "endSessions" => admin::end_sessions().await,
        "currentOp" => admin::current_op().await,
        "serverStatus" => admin::server_status().await,
        _ => {
            let mut resp = Document::new();
            resp.insert("ok".to_string(), Value::Number(0.into()));
            resp.insert(
                "errmsg".to_string(),
                Value::String(format!("unknown command: {}", cmd_name)),
            );
            Ok(resp)
        }
    }
}

fn command_name_from_doc(doc: &Document) -> String {
    doc.keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}
