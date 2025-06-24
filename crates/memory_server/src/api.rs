use actix_web::{web, HttpResponse};
use chrono::Utc;
use uuid::Uuid;

use crate::embeddings::EmbeddingService;
use crate::error::Result;
use crate::models::{
    CategorizeMemoryRequest, CategorizeMemoryResponse, CreateMemoryRequest,
    CreateMemoryResponse, Memory, SearchMemoryRequest, SearchMemoryResponse,
};
use crate::storage::MemoryStorage;

pub async fn store_memory(
    data: web::Data<MemoryStorage>,
    req: web::Json<CreateMemoryRequest>,
) -> Result<HttpResponse> {
    let embedding_service = EmbeddingService::new();
    let embedding = embedding_service
        .generate_embedding(&req.content)
        .await?;

    let memory = Memory {
        id: Uuid::new_v4(),
        content: req.content.clone(),
        metadata: req.metadata.clone(),
        category: req.category.clone(),
        embedding,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let id = memory.id;
    let created_at = memory.created_at;

    data.store(memory).await?;

    Ok(HttpResponse::Created().json(CreateMemoryResponse { id, created_at }))
}

pub async fn search_memory(
    data: web::Data<MemoryStorage>,
    req: web::Json<SearchMemoryRequest>,
) -> Result<HttpResponse> {
    let limit = req.limit.unwrap_or(10).min(100);
    let results = data
        .search(&req.query, limit, req.category.as_deref())
        .await?;

    let total = results.len();

    Ok(HttpResponse::Ok().json(SearchMemoryResponse { results, total }))
}

pub async fn get_memory(
    data: web::Data<MemoryStorage>,
    path: web::Path<Uuid>,
) -> Result<HttpResponse> {
    let memory = data.get(path.into_inner()).await?;
    Ok(HttpResponse::Ok().json(memory))
}

pub async fn categorize_memory(
    data: web::Data<MemoryStorage>,
    req: web::Json<CategorizeMemoryRequest>,
) -> Result<HttpResponse> {
    let (category, confidence) = data.categorize(&req.content).await?;

    Ok(HttpResponse::Ok().json(CategorizeMemoryResponse {
        category,
        confidence,
    }))
}