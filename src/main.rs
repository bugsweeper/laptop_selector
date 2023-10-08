use axum::{
    response::Html,
    routing::{get, post},
    Extension, Router,
};
use laptop_selector::{connect, get_laptops, LaptopView};
use minijinja::render;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

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

async fn laptop_request(
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
async fn default_laptop_request(
    laptops: Extension<Arc<Vec<LaptopView>>>,
    maximums: Extension<(i64, i64)>,
) -> Html<String> {
    laptop_request(laptops, maximums, String::from("cpu=100&gpu=0&quantity=10")).await
}

#[tokio::main]
async fn main() {
    let pool = Arc::new(connect().await);
    let laptops = Arc::new(get_laptops(pool).await.unwrap());
    let max_scores = (
        laptops
            .iter()
            .max_by_key(|laptop| laptop.cpu_score)
            .unwrap()
            .cpu_score,
        laptops
            .iter()
            .max_by_key(|laptop| laptop.gpu_score)
            .unwrap()
            .gpu_score,
    );

    let app = Router::new()
        .route("/laptop_selector", post(laptop_request))
        .route("/laptop_selector", get(default_laptop_request))
        .layer(Extension(laptops))
        .layer(Extension(max_scores));

    let addr = SocketAddr::from(([127, 0, 0, 1], 80));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
