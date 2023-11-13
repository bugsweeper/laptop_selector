use laptop_selector::{connect, get_laptops, Error};
use prettytable::{row, Table};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let pool = Arc::new(connect().await);
    let mut laptops = get_laptops(pool).await?;
    laptops.sort_by_key(|laptop| laptop.price * 1000 / (laptop.cpu_score + 1));
    let mut table = Table::new();
    table.add_row(row!["Score", "Price", "Name", "Url"]);
    for laptop in laptops {
        table.add_row(row![
            laptop.cpu_score,
            laptop.price,
            laptop.description.split('/').next().unwrap().trim(),
            laptop.url
        ]);
    }
    table.printstd();

    Ok(())
}
