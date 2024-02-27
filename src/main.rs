use std::error::Error;
use std::fs::File;
use std::time::Duration;

use async_recursion::async_recursion;
use futures::prelude::*;
use native_tls::TlsConnector;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use url::Url;

pub mod gemini_web;

use gemini_web::{GeminiHeader, GeminiResponse, GeminiWeb};

const TIMEOUT: Duration = Duration::from_secs(1);
const MAX_REDIRECT: usize = 256;

#[async_recursion]
async fn gemini_get_recursion(
    url: &Url,
    redirect_count: usize,
) -> Result<GeminiResponse, Box<dyn Error>> {
    if redirect_count > MAX_REDIRECT {
        return Err("Max redirect {MAX_REDIRECT} reached".into());
    }
    let domain = url.domain().unwrap();
    let domain_port = domain.to_owned() + ":1965";
    // Setup SSL
    let stream = timeout(TIMEOUT, TcpStream::connect(domain_port)).await??;
    let cx = TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        // library says it uses the default system certs but doesn't work for me
        .danger_accept_invalid_certs(true)
        .build()?;
    let cx = tokio_native_tls::TlsConnector::from(cx);
    // Connect to base url and query the gemini page
    let mut stream = timeout(TIMEOUT, cx.connect(domain, stream)).await??;
    timeout(
        TIMEOUT,
        stream.write_all((url.to_string() + "\r\n").as_bytes()),
    )
    .await??;
    // TODO: parse header in a buf instead of trying to put the whole response in a string
    // (some response contain binary data like images but still have a valid header)
    let mut response_bytes = Vec::new();
    timeout(TIMEOUT, stream.read_to_end(&mut response_bytes)).await??;

    let response = GeminiResponse::new(&response_bytes, url)?;
    use GeminiHeader::*;
    match response.header {
        Success(ref mime) if mime.essence_str() == "text/gemini" => Ok(response),
        Success(mime) => Err(format!("invalid MIME {mime}").into()),
        Redirect(url) => {
            println!("Following redirect to {url}");
            gemini_get_recursion(&url, redirect_count + 1).await
        }
        _ => Err(format!("invalid header type {:?}", response.header).into()),
    }
}

async fn gemini_get(url: &Url) -> Result<GeminiResponse, Box<dyn Error>> {
    gemini_get_recursion(url, 0).await
}

const CHANNEL_LEN: usize = 200;

async fn visit_url(
    mut web: GeminiWeb,
    urls: Vec<Url>,
    mut stop: oneshot::Receiver<()>,
) -> Result<GeminiWeb, Box<dyn Error>> {
    let (url_tx, mut url_rx) = mpsc::channel(CHANNEL_LEN);
    let (response_tx, mut response_rx) = mpsc::channel(CHANNEL_LEN);

    let url_tx_clone = url_tx.clone();
    let urls_clone = urls.clone();
    tokio::spawn(async move {
        for url in urls_clone {
            println!("Add {} to urls", url);
            url_tx_clone.send(url.clone()).await.unwrap();
        }
    });

    let _querier = tokio::spawn(async move {
        println!("Start querier");

        let stream = async_stream::stream! {
            while let Some(url) = url_rx.recv().await {
                yield url;
            }
        };
        stream
            .for_each_concurrent(Some(10), |url| async {
                let response = match gemini_get(&url).await {
                    Ok(r) => r,
                    Err(e) => {
                        println!("Error gemini_get for {}: {}", url, e);
                        return;
                    }
                };
                let _ = response_tx.send((url, response)).await;
            })
            .await;
    });

    loop {
        tokio::select! {
            _ = &mut stop => {
                drop(url_tx);
                break;
            },
            Some((url, response)) = response_rx.recv() => {
                web.visited.insert(url.clone());
                let node_id = web.add_node(&url);
                let urls = response.gemini_urls();
                let urls: Vec<Url> = urls
                    .iter()
                    .filter(|u| !web.visited.contains(u))
                    .cloned()
                    .collect();
                let _node_ids = web.add_urls(node_id, &urls);
                for u in urls {
                    if url_tx.send(u.clone()).await.is_err() {
                        break;
                    }
                }
            },
        };
    }
    Ok(web)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // println!("{:?}", gemini_get(&Url::parse("gemini://idiomdrottning.org/wotch")?).await);
    // return Ok(());

    let mut web = match File::open("web.bincode") {
        Ok(reader) => bincode::deserialize_from(reader)?,
        _ => GeminiWeb::new(),
    };
    let mut unvisited_urls = web.unvisited();
    if unvisited_urls.is_empty() {
        unvisited_urls = std::fs::read_to_string("seeds")?
            .lines()
            .map(Url::parse)
            .collect::<Result<_, _>>()?;
    }
    let (stop_tx, stop_rx) = oneshot::channel();

    let crawler_task = tokio::spawn(async move {
        web = visit_url(web, unvisited_urls, stop_rx).await.unwrap();
        // println!("Node count: {}", graph.node_count());
        // println!("Edge count: {}", graph.edge_count());
        println!("Saving web to file");
        let web_file = File::create("web.bincode").unwrap();
        bincode::serialize_into(web_file, &web).unwrap();
        web.to_dot("web.svg").unwrap();
    });

    tokio::signal::ctrl_c().await?;
    println!("Stopping");
    stop_tx.send(()).unwrap();
    crawler_task.await?;

    Ok(())
}
