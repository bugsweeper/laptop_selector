use axum::{response::Html, routing::post, Extension, Router};
use fantoccini::error::CmdError;
use minijinja::render;
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

const PAGE_TEMPLATE: &str = r#"
<!doctype html>

<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">

  <title>Laptops ordered list</title>
  <meta name="description" content="By default laptops ordered by cpu benchmark">
  <meta name="author" content="Vitalii">
</head>

<body>
    <form action="/laptop_selector" method="post">
        <p> CPU priority: </p>
        <div><input type="range" id="cpu" name="cpu" min="0" max="1000" value="{{param.cpu}}" /></div>

        <p> GPU priority: </p>
        <div><input type="range" id="gpu" name="gpu" min="0" max="1000" value="{{param.gpu}}" /></div>

        <p> Laptop quantity: </p>
        <select id="quantity" name="quantity">
            <option value="5">5</option>
            <option value="10">10</option>
            <option value="20">20</option>
            <option value="50">50</option>
        </select>
        <input type="submit">
        <script>
            document.getElementById('quantity').value = '{{param.quantity}}';
        </script>
    </form>
    <p>Laptops:</p>
    <table>
        <tr>
            <th>Score</th>
            <th>Price</th>
            <th>Info</th>
        </tr>
        {% for laptop in laptops %}
        <tr>
            <td>{{laptop.total_score}}</td>
            <td>{{laptop.laptop.price}}</td>
            <td><a href="{{laptop.laptop.url}}">{{laptop.laptop.description}}</a></td>
        </tr>
        {% endfor %}
    </table>
</body>
</html>
"#;

#[derive(Serialize, Deserialize, Default)]
struct LaptopPriorities {
    cpu: i64,
    gpu: i64,
    quantity: usize,
}

#[derive(Serialize)]
struct ScoredLaptop<'a> {
    laptop: &'a LaptopView,
    total_score: i64,
}

async fn laptop_request_handler(
    Extension(laptops): Extension<Arc<Vec<LaptopView>>>,
    Extension(maximums): Extension<(i64, i64)>,
    params: String,
) -> Html<String> {
    let params: LaptopPriorities = serde_urlencoded::from_str(&params).unwrap_or_default();
    let mut sorted_laptops = laptops
        .as_ref()
        .iter()
        .map(|laptop| ScoredLaptop {
            laptop,
            total_score: laptop.cpu_score * params.cpu / maximums.0
                + laptop.gpu_score * params.gpu / maximums.1,
        })
        .collect::<Vec<_>>();
    sorted_laptops.sort_by_key(|laptop| laptop.laptop.price * 1000 / (laptop.total_score + 1));
    let page = render!(PAGE_TEMPLATE,param=>params,laptops=>&sorted_laptops[0..params.quantity]);
    Html(page)
}

async fn default_laptop_request_handler(
    laptops: Extension<Arc<Vec<LaptopView>>>,
    maximums: Extension<(i64, i64)>,
) -> Html<String> {
    laptop_request_handler(laptops, maximums, String::from("cpu=100&gpu=0&quantity=10")).await
}

pub async fn prepare_laptop_requests_router() -> Router {
    let pool = Arc::new(connect().await);
    let laptops = Arc::new(get_laptops(pool).await.unwrap());
    let max_scores = (
        laptops.iter().map(|laptop| laptop.cpu_score).max().unwrap(),
        laptops.iter().map(|laptop| laptop.gpu_score).max().unwrap(),
    );

    Router::new()
        .route(
            "/laptop_selector",
            post(laptop_request_handler).get(default_laptop_request_handler),
        )
        .layer(Extension(laptops))
        .layer(Extension(max_scores))
}
