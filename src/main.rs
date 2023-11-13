use laptop_selector::prepare_laptop_requests_router;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 80));
    axum::Server::bind(&addr)
        .serve(prepare_laptop_requests_router().await.into_make_service())
        .await
        .unwrap();
}
