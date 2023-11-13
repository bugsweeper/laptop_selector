use fantoccini::error::CmdError;
use serde::{Deserialize, Serialize};
use sqlx::{migrate::MigrateDatabase, Sqlite, SqlitePool};
use std::sync::Arc;

const DB_URL: &str = "sqlite://laptops.db";

pub async fn connect() -> SqlitePool {
    if Sqlite::database_exists(DB_URL).await.unwrap_or(false) {
        SqlitePool::connect(DB_URL).await.unwrap()
    } else {
        println!("Creating database {DB_URL}");
        Sqlite::create_database(DB_URL)
            .await
            .expect("database creation error");

        let db = SqlitePool::connect(DB_URL)
            .await
            .expect("database connection error");
        sqlx::migrate!()
            .run(&db)
            .await
            .expect("tables creation error");
        db
    }
}

#[derive(Debug, Deserialize)]
pub struct Cpu {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub score: i64,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("SQLx error occurred: {0}")]
    SqlxError(#[from] sqlx::Error),

    #[error("WebDriver error occured: {0}")]
    WebDriver(#[from] CmdError),

    #[error("UrlDecode error occured: {0}")]
    UrlDecode(#[from] serde_urlencoded::de::Error),

    #[error("Parse int error occured: {0}")]
    ParseInt(#[from] std::num::ParseIntError),

    #[error("Read config error occured: {0}")]
    ConfigError(#[from] config::ConfigError),
}

pub async fn get_cpus(pool: Arc<SqlitePool>) -> Result<Vec<Cpu>, Error> {
    let mut from_base = sqlx::query_as!(
        Cpu,
        "
            SELECT * FROM cpu ORDER BY id ASC;
        "
    )
    .fetch_all(pool.as_ref())
    .await?;
    for cpu in &mut from_base {
        cpu.name = cpu.name.split('@').next().unwrap().trim().to_owned();
    }
    Ok(from_base)
}

pub async fn get_gpus(pool: Arc<SqlitePool>) -> Result<Vec<Cpu>, Error> {
    let mut from_base = sqlx::query_as!(
        Cpu,
        "
            SELECT * FROM gpu ORDER BY id ASC;
        "
    )
    .fetch_all(pool.as_ref())
    .await?;
    for cpu in &mut from_base {
        cpu.name = cpu.name.split(',').next().unwrap().trim().to_owned();
    }
    Ok(from_base)
}

#[derive(PartialEq, Serialize)]
pub struct LaptopView {
    pub id: i64,
    pub image: String,
    pub description: String,
    pub composition: String,
    pub url: String,
    pub price: i64,
    pub cpu_id: i64,
    pub gpu_id: i64,
    pub cpu_score: i64,
    pub gpu_score: i64,
    /// for debug fuzzy comparison  purposes
    pub cpu_name: String,
    pub gpu_name: String,
}

pub async fn get_laptops(pool: Arc<SqlitePool>) -> Result<Vec<LaptopView>, Error> {
    Ok(sqlx::query_as!(
        LaptopView,
        "
            SELECT laptop.id, laptop.image, laptop.description, 
                laptop.composition, laptop.url, laptop.price, 
                laptop.cpu_id, laptop.gpu_id,
                cpu.score as cpu_score, gpu.score as gpu_score,
                cpu.name as cpu_name, gpu.name as gpu_name 
            FROM laptop
                JOIN cpu ON laptop.cpu_id = cpu.id
                JOIN gpu on laptop.gpu_id = gpu.id;
        "
    )
    .fetch_all(pool.as_ref())
    .await?)
}
