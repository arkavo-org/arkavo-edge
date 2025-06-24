use actix_web::{middleware, web, App, HttpServer};
use anyhow::Result;
use env_logger::Env;
use log::info;

mod api;
mod embeddings;
mod error;
mod models;
mod storage;

use api::{categorize_memory, get_memory, search_memory, store_memory};
use storage::MemoryStorage;

#[actix_web::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    info!("Starting Arkavo Memory Server...");

    let storage = web::Data::new(MemoryStorage::new().await?);

    info!("Memory Server starting on http://localhost:8080");

    HttpServer::new(move || {
        App::new()
            .app_data(storage.clone())
            .wrap(middleware::Logger::default())
            .route("/memory", web::post().to(store_memory))
            .route("/memory/search", web::post().to(search_memory))
            .route("/memory/categorize", web::post().to(categorize_memory))
            .route("/memory/{id}", web::get().to(get_memory))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await?;

    Ok(())
}