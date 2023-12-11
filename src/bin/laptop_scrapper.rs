use fantoccini::elements::Element;
use fantoccini::error::CmdError;
use fantoccini::{ClientBuilder, Locator};
use futures::{future::BoxFuture, FutureExt};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use laptop_selector::{connect, get_cpus, get_gpus, get_laptops, Cpu, Error, LaptopView};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

struct LaptopWithNoComposition {
    id: i64,
    image: String,
    description: String,
    price: i64,
}

enum ParserType {
    CpuBenchmark,
    GpuBenchmark,
    /// bool parameter: add walking on paginator (should be only once, to avoid recursion)
    /// then goes two arrays of Cpu with cpu and gpu collections
    RozetkaLaptopList(bool, Arc<Vec<LaptopView>>, Arc<Vec<Cpu>>, Arc<Vec<Cpu>>),
    /// Partialy gathered info from common list, get composition from products page
    RozetkaLaptopDescription(LaptopWithNoComposition, Arc<Vec<Cpu>>, Arc<Vec<Cpu>>),
    RozetkaLaptopListWithApiCalls(Arc<Vec<LaptopView>>, Arc<Vec<Cpu>>, Arc<Vec<Cpu>>),
}

fn get_best_match(devices: &Vec<&str>, cpus: &[Cpu]) -> usize {
    let matcher = SkimMatcherV2::default();
    let mut cpu_index = 0;
    let mut best_score = 0;
    for (index, cpu) in cpus.iter().enumerate() {
        for device in devices {
            if let Some(score) = matcher.fuzzy_match(device, &cpu.name) {
                if score > best_score {
                    best_score = score;
                    cpu_index = index;
                }
            }
        }
    }
    cpu_index
}

async fn try_load_by_element(
    element: &Element,
    repeat: bool,
    css_selector: &str,
) -> Result<Element, CmdError> {
    let mut subelement = element.find(Locator::Css(css_selector)).await;
    if repeat {
        for _ in 0..3 {
            if subelement.is_err() {
                std::thread::sleep(std::time::Duration::from_secs(1));
                subelement = element.find(Locator::Css(css_selector)).await;
            } else {
                break;
            }
        }
    }
    subelement
}

async fn try_load_by_client(
    element: &fantoccini::Client,
    css_selector: &str,
) -> Result<Element, CmdError> {
    let mut subelement = element.find(Locator::Css(css_selector)).await;
    for _ in 0..3 {
        if subelement.is_err() {
            std::thread::sleep(std::time::Duration::from_secs(5));
            subelement = element.find(Locator::Css(css_selector)).await;
        } else {
            break;
        }
    }
    subelement
}

const DATA_FETCHER: &'static str = r#"
    const [request, callback] = arguments;
    fetch(`https://xl-catalog-api.rozetka.com.ua/v4/goods/` + request)
    .then(data => {
        callback(data.json())
    })
"#;

async fn process_page_ajax(
    number: u64,
    client: &fantoccini::Client,
    pool: &Arc<SqlitePool>,
    cpus: &Arc<Vec<Cpu>>,
    gpus: &Arc<Vec<Cpu>>,
) -> u64 {
    println!("Parsing page {number}");
    let result = &client
        .execute_async(
            DATA_FETCHER,
            vec![json!(format!(
                "get?front-type=xl&country=UA&lang=ua&page={number}&category_id=80004"
            ))],
        )
        .await
        .unwrap()["data"];
    let total_pages = result["total_pages"].as_u64().unwrap_or(0);
    let ids = result["ids"].as_array().unwrap();
    let mut request = ids.into_iter().map(|id| id.as_u64().unwrap().to_string()).fold(String::from("getDetails?country=UA&lang=ua&with_groups=1&with_docket=1&goods_group_href=1&product_ids="), |a, b| a + &b[..] + ",");
    request.pop();
    let result = &client
        .execute_async(DATA_FETCHER, vec![json!(request)])
        .await
        .unwrap()["data"];
    let laptops = result.as_array().unwrap();
    for laptop in laptops {
        let laptop = laptop.as_object().unwrap();
        let id = laptop["id"].as_i64().unwrap();
        let description = &laptop["title"].as_str().unwrap();
        let price = laptop["price"].as_i64().unwrap();
        let url = &laptop["href"].as_str().unwrap();
        let composition = &laptop["docket"].as_str().unwrap_or_else(|| {
            if let Some(array) = &laptop["docket"].as_array() {
                if let Some(object) = array[0].as_object() {
                    object["value_title"].as_str().unwrap_or("")
                } else {
                    println!("Object not found in {laptop:#?}");
                    ""
                }
            } else {
                println!("Array not found in {laptop:#?}");
                ""
            }
        });
        let image = &laptop["image_main"].as_str().unwrap_or("");

        let devices = composition
            .split('/')
            .map(|device| device.split('(').next().unwrap())
            .map(|device| device.split('(').next().unwrap())
            .map(str::trim)
            .collect();
        let cpu = &cpus[get_best_match(&devices, &cpus)];
        let gpu = &gpus[get_best_match(&devices, &gpus)];

        if composition.is_empty() || image.is_empty() {
            println!("Not full info in {laptop:#?}");
        }

        if composition.is_empty() {
            sqlx::query!(
                "INSERT INTO laptop(
                        id,
                        image,
                        description,
                        url,
                        price,
                        cpu_id,
                        gpu_id
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT(id) DO
                    UPDATE SET
                        image=excluded.image,
                        description=excluded.description,
                        url=excluded.url,
                        price=excluded.price,
                        cpu_id=excluded.cpu_id,
                        gpu_id=excluded.gpu_id;
                    ",
                id,
                image,
                description,
                url,
                price,
                cpu.id,
                gpu.id
            )
            .execute(pool.as_ref())
            .await
            .unwrap();
        } else {
            sqlx::query!(
                "INSERT OR REPLACE INTO laptop(
                        id,
                        image,
                        description,
                        composition,
                        url,
                        price,
                        cpu_id,
                        gpu_id
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                id,
                image,
                description,
                composition,
                url,
                price,
                cpu.id,
                gpu.id
            )
            .execute(pool.as_ref())
            .await
            .unwrap();
        }
    }

    total_pages
}

fn parse(
    webdriver: String,
    uri: String,
    parser_type: ParserType,
    pool: Arc<SqlitePool>,
    semaphore: Arc<Semaphore>,
) -> BoxFuture<'static, std::result::Result<(), Error>> {
    async move {
        let permit = semaphore.acquire().await.unwrap();

        // Open new window, and load page
        let c = ClientBuilder::native()
            // .connect("http://127.0.0.1:4444")    // gekodriver
            .connect(&webdriver) // chromedriver
            .await
            .expect("failed to connect to WebDriver");
        c.goto(&uri).await?;
        c.maximize_window().await?;

        match parser_type {
            ParserType::CpuBenchmark => {
                // At least check two times, to ensure JS loading is not active anymore
                let mut row_count = 0;
                let mut rows = c.find_all(Locator::Css("#cputable tbody tr")).await?;
                while row_count == 0 || row_count != rows.len() {
                    row_count = rows.len();
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    rows = c.find_all(Locator::Css("#cputable tbody tr")).await?;
                }
                sqlx::query!(
                    r#"INSERT INTO cpu(id, name, url, score) VALUES (0, "Unknown cpu", "", 0)"#,
                )
                .execute(pool.as_ref())
                .await?;

                for row in rows {
                    let cells = row.find_all(Locator::Css("td")).await?;
                    // avoid repeated header
                    if cells.len() < 2 {
                        continue;
                    }
                    let link = &cells[0].find(Locator::Css("a")).await?;
                    let href = link.attr("href").await?.unwrap_or_default();
                    let url = format!("https://www.cpubenchmark.net/{href}").replace("_lookup", "");
                    let id = &serde_urlencoded::from_str::<HashMap<String, String>>(&href)?["id"];
                    let name = link.text().await.unwrap_or_default();
                    let score = cells[1]
                        .text()
                        .await
                        .unwrap_or_default()
                        .replace(',', "")
                        .parse::<u32>()
                        .unwrap_or_default();

                    sqlx::query!(
                        "INSERT INTO cpu(id, name, url, score) VALUES ($1, $2, $3, $4)",
                        id,
                        name,
                        url,
                        score
                    )
                    .execute(pool.as_ref())
                    .await?;
                }
                println!("CPU benchmarks dump complete");
            }
            ParserType::GpuBenchmark => {
                // At least check two times, to ensure JS loading is not active anymore
                let mut row_count = 0;
                let mut rows = c.find_all(Locator::Css("#cputable tbody tr")).await?;
                while row_count == 0 || row_count != rows.len() {
                    row_count = rows.len();
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    rows = c.find_all(Locator::Css("#cputable tbody tr")).await?;
                }
                sqlx::query!(
                    r#"INSERT INTO gpu(id, name, url, score) VALUES (0, "Unknown gpu", "", 0)"#,
                )
                .execute(pool.as_ref())
                .await?;

                for row in rows {
                    let cells = row.find_all(Locator::Css("td")).await?;
                    // avoid repeated header
                    if cells.len() < 2 {
                        continue;
                    }
                    let link = &cells[0].find(Locator::Css("a")).await?;
                    let href = link.attr("href").await?.unwrap_or_default();
                    let url = format!("https://www.videocardbenchmark.net/{href}")
                        .replace("video_lookup", "gpu");
                    let id = &serde_urlencoded::from_str::<HashMap<String, String>>(&href)?["id"];
                    let name = link.text().await.unwrap_or_default();
                    let score = cells[1]
                        .text()
                        .await
                        .unwrap_or_default()
                        .replace(',', "")
                        .parse::<u32>()
                        .unwrap_or_default();

                    sqlx::query!(
                        "INSERT INTO gpu(id, name, url, score) VALUES ($1, $2, $3, $4)",
                        id,
                        name,
                        url,
                        score
                    )
                    .execute(pool.as_ref())
                    .await?;
                }
                println!("GPU benchmarks dump complete");
            }
            ParserType::RozetkaLaptopList(spawn_from_paginator, laptops, cpus, gpus) => {
                // At least check two times, to ensure JS loading is not active anymore
                let mut laptop_count = 0;
                let mut laptop_elements = c.find_all(Locator::Css(".catalog-grid__cell")).await?;
                while laptop_count == 0 || laptop_count != laptop_elements.len() {
                    laptop_count = laptop_elements.len();
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    laptop_elements = c.find_all(Locator::Css(".catalog-grid__cell")).await?;
                }
                let mut set = tokio::task::JoinSet::new();
                let mut first_time = true;
                for laptop in laptop_elements {
                    let id: i64 = laptop
                        .find(Locator::Css("div.g-id"))
                        .await?
                        .prop("innerText")
                        .await?
                        .unwrap_or_default()
                        .parse()?;
                    let image = laptop
                        .find(Locator::Css("img"))
                        .await?
                        .attr("src")
                        .await?
                        .unwrap_or_default();
                    let description = laptop
                        .find(Locator::Css(".goods-tile__title"))
                        .await?
                        .text()
                        .await?;
                    let composition = if let Ok(element) = try_load_by_element(&laptop, first_time, "p.goods-tile__description_type_text").await {
                            element.text().await?
                        } else if let Ok(element) = try_load_by_element(&laptop, first_time, "span.goods-tile__description-control").await {
                            element.text().await?.replace("•", "/")
                        } else if let Ok(element) = try_load_by_element(&laptop, first_time, ".goods-tile__hidden-content").await {
                            element.text().await?
                        } else {
                            String::new()
                        };
                    let price: i64 = laptop
                        .find(Locator::Css(".goods-tile__price-value"))
                        .await?
                        .text()
                        .await?
                        .chars()
                        .filter(char::is_ascii_digit)
                        .collect::<String>()
                        .parse()?;
                    let url = laptop
                        .find(Locator::Css("a.goods-tile__heading"))
                        .await?
                        .attr("href")
                        .await?
                        .unwrap_or_default();
                    // println!("url: {url}");

                    let devices = composition.split('/').map(|device| device.split('(').next().unwrap()).map(|device|device.split('(').next().unwrap()).map(str::trim).collect();
                    let cpu = &cpus[get_best_match(&devices, &cpus)];
                    let gpu = &gpus[get_best_match(&devices, &gpus)];

                    first_time = false;
                    if composition.is_empty() {
                        sqlx::query!(
                            "INSERT INTO laptop(
                                id,
                                image,
                                description,
                                url,
                                price,
                                cpu_id,
                                gpu_id
                            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                            ON CONFLICT(id) DO
                            UPDATE SET
                                image=excluded.image,
                                description=excluded.description,
                                url=excluded.url,
                                price=excluded.price,
                                cpu_id=excluded.cpu_id,
                                gpu_id=excluded.gpu_id;
                            ",
                            id,
                            image,
                            description,
                            url,
                            price,
                            cpu.id,
                            gpu.id
                        )
                        .execute(pool.as_ref())
                        .await?;

                        if let Some(laptop) = laptops.iter().find(|laptop| laptop.id == id) {
                            // Do not erase fullfilled information
                            if laptop.composition.is_some() {
                                println!("Skip loading composition of {}", laptop.description);
                                continue;
                            }
                        }
                        let pool_clone = pool.clone();
                        set.spawn(parse(
                            webdriver.clone(),
                            url.clone(),
                            ParserType::RozetkaLaptopDescription(
                                LaptopWithNoComposition {
                                    id,
                                    image: image.clone(),
                                    description: description.clone(),
                                    price,
                                },
                                cpus.clone(),
                                gpus.clone(),
                            ),
                            pool_clone,
                            semaphore.clone()
                        ));
                    } else {
                        println!("Matched composition:{composition:#?}\nwith cpu: {cpu:#?}\nand gpu: {gpu:#?}");
                        sqlx::query!(
                            "INSERT OR REPLACE INTO laptop(
                                id,
                                image,
                                description,
                                composition,
                                url,
                                price,
                                cpu_id,
                                gpu_id
                            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                            id,
                            image,
                            description,
                            composition,
                            url,
                            price,
                            cpu.id,
                            gpu.id
                        )
                        .execute(pool.as_ref())
                        .await?;
                    }
                }
                if spawn_from_paginator {
                    let pages = c.find_all(Locator::Css("a.pagination__link")).await?;
                    let mut max_page = 0;
                    for page in pages {
                        if let Some(page_param) = page.attr("href").await?.unwrap_or_default().split('/').rev().nth(1) {
                            let page_number = page_param.split('=').last().unwrap().parse::<i32>()?;
                            if page_number > max_page {
                                max_page = page_number;
                            }
                        }
                    }
                    if max_page > 1 {
                        for i in 2..=max_page {
                            set.spawn(parse(
                                webdriver.clone(),
                                format!("{uri}page={i}/"),
                                ParserType::RozetkaLaptopList(false, laptops.clone(), cpus.clone(), gpus.clone()),
                                pool.clone(),
                                semaphore.clone(),
                            ));
                        }
                    }
                }
                // No pages to open, nobody should wait anymore
                c.close_window().await?;
                drop(permit);
                // for those, who has no composition info
                while let Some(result) = set.join_next().await {
                    if result.is_err() {
                        println!("{result:#?}");
                    }
                }
            }
            ParserType::RozetkaLaptopDescription(laptop, cpus, gpus) => {
                let composition = if let Ok(element) = try_load_by_client(&c, ".product-about__brief").await {
                    element.text().await?
                } else {
                    try_load_by_client(&c, "ul.characteristics-simple__sub-list span.ng-star-inserted").await?.text().await?.replace("•", "/")
                };

                let devices = composition.split('/').map(|device| device.split('(').next().unwrap()).map(|device|device.split('(').next().unwrap()).map(str::trim).collect();
                let cpu = &cpus[get_best_match(&devices, &cpus)];
                let gpu = &gpus[get_best_match(&devices, &gpus)];

                sqlx::query!(
                    "INSERT OR REPLACE INTO laptop(
                        id,
                        image,
                        description,
                        composition,
                        url,
                        price,
                        cpu_id,
                        gpu_id
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                    laptop.id,
                    laptop.image,
                    laptop.description,
                    composition,
                    uri,
                    laptop.price,
                    cpu.id,
                    gpu.id
                )
                .execute(pool.as_ref())
                .await?;
                println!("Loaded composition of {}", laptop.description);
            }
            ParserType::RozetkaLaptopListWithApiCalls(laptops, cpus, gpus) => {
                let total_pages = process_page_ajax(1, &c, &pool, &cpus, &gpus).await;
                for i in 2..=total_pages {
                    let _ = process_page_ajax(i, &c, &pool, &cpus, &gpus).await;
                }
            }
        }

        c.close_window().await?;
        c.close().await?;
        Ok(())
    }
    .map(|result| {
        if result.is_err() {
            println!("{result:#?}");
        }
        result
    })
    .boxed()
}

#[derive(Deserialize, Serialize)]
pub struct WebDriverSettings {
    pub host: String,
    pub port: u16,
}

impl Default for WebDriverSettings {
    fn default() -> Self {
        Self {
            host: String::from("127.0.0.1"),
            port: 9515,
        }
    }
}

impl WebDriverSettings {
    fn connection_url(self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

pub fn get_configuration() -> Result<WebDriverSettings, config::ConfigError> {
    config::Config::builder()
        .add_source(config::Config::try_from(&WebDriverSettings::default()).unwrap())
        .add_source(config::File::with_name("webdriver.yaml"))
        .add_source(
            config::Environment::with_prefix("LAPTOP_SCRAPPER")
                .try_parsing(true)
                .separator("_"),
        )
        .build()?
        .try_deserialize()
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let webdriver_url = get_configuration()?.connection_url();
    let pool = Arc::new(connect().await);
    let semaphore = Arc::new(Semaphore::new(10));

    let mut set = tokio::task::JoinSet::new();

    let mut cpus = Arc::new(get_cpus(pool.clone()).await?);
    if cpus.is_empty() {
        set.spawn(parse(
            webdriver_url.clone(),
            String::from("https://www.cpubenchmark.net/cpu_list.php"),
            ParserType::CpuBenchmark,
            pool.clone(),
            semaphore.clone(),
        ));
    }

    let mut gpus = Arc::new(get_gpus(pool.clone()).await?);
    if gpus.is_empty() {
        set.spawn(parse(
            webdriver_url.clone(),
            String::from("https://www.videocardbenchmark.net/gpu_list.php"),
            ParserType::GpuBenchmark,
            pool.clone(),
            semaphore.clone(),
        ));
    }

    while let Some(result) = set.join_next().await {
        if result.is_err() {
            println!("{result:#?}");
        }
    }

    // All data is saved to database
    if cpus.is_empty() {
        cpus = get_cpus(pool.clone()).await?.into();
    }

    if gpus.is_empty() {
        gpus = get_gpus(pool.clone()).await?.into();
    }

    let laptops = Arc::new(get_laptops(pool.clone()).await?);

    set.spawn(parse(
        webdriver_url.clone(),
        String::from("https://rozetka.com.ua/ua/notebooks/c80004/"),
        ParserType::RozetkaLaptopListWithApiCalls(laptops, cpus, gpus),
        pool,
        semaphore.clone(),
    ));

    if let Err(err) = set.join_next().await.transpose() {
        println!("{err:#?}");
    };

    Ok(())
}
