use std::cell::RefCell;
use std::error::Error;
use std::fs::File;
use std::rc::Rc;
use std::time::Duration;

use async_recursion::async_recursion;
use futures::prelude::*;
use futures::stream::FuturesUnordered;
use native_tls::TlsConnector;
use petgraph::graph::NodeIndex;
use tokio;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use url::Url;

pub mod gemini_web;

use gemini_web::{GeminiHeader, GeminiResponse, GeminiWeb};

const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_REDIRECT: usize = 256;

#[async_recursion(?Send)]
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
    let mut response = String::new();
    timeout(TIMEOUT, stream.read_to_string(&mut response)).await??;

    let response = GeminiResponse::new(&response, url)?;
    use GeminiHeader::*;
    match response.header {
        Success(ref mime) if mime.essence_str() == "text/gemini" => Ok(response),
        Success(mime) => Err(format!("invalid MIME {mime}").into()),
        Redirect(url) => {
            eprintln!("Following redirect to {url}");
            gemini_get_recursion(&url, redirect_count + 1).await
        }
        _ => Err(format!("invalid header type {:?}", response.header).into()),
    }
}

async fn gemini_get(url: &Url) -> Result<GeminiResponse, Box<dyn Error>> {
    gemini_get_recursion(url, 0).await
}

#[async_recursion(?Send)]
async fn visit_url_recursion(
    base_url: Url,
    base_node_id: NodeIndex,
    web: Rc<RefCell<GeminiWeb>>,
    depth: usize,
) -> Result<(), Box<dyn Error>> {
    // tokio::time::interval is annoying because putting it in a RefCell causes runtime crash
    tokio::time::sleep(Duration::from_millis(100)).await;
    if depth == 0 || web.borrow_mut().try_visit(&base_url) {
        return Ok(());
    }
    eprintln!("Visiting {}", base_url.to_string());
    let response = gemini_get(&base_url).await?;
    println!("{}", response.body);

    let urls = response.gemini_urls();
    let node_ids = web.borrow_mut().add_urls(base_node_id, &urls);

    let mut fs = urls
        .iter()
        .zip(node_ids)
        .map(|(url, node_id)| visit_url_recursion(url.clone(), node_id, web.clone(), depth - 1))
        .collect::<FuturesUnordered<_>>();
    while let Some(r) = fs.next().await {
        match r {
            Ok(_response) => (),
            Err(e) => eprintln!("Visit url error: {}", e),
        }
    }
    Ok(())
}

async fn visit_url(
    web: GeminiWeb,
    base_url: Url,
    depth: usize,
) -> Result<GeminiWeb, Box<dyn Error>> {
    let web = Rc::new(RefCell::new(web));
    let base_node_id = web.borrow_mut().add_node(&base_url);
    visit_url_recursion(base_url, base_node_id, web.clone(), depth).await?;
    Ok(web.take()) // FIXME: understand why into_inner() doesn't work here
}

const BASE_URL: &str = "gemini://makeworld.space/amfora-wiki/";
const DEPTH: usize = 3;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut web = match File::open("web.bincode") {
        Ok(reader) => bincode::deserialize_from(reader)?,
        _ => GeminiWeb::new(),
    };
    let mut unvisited_urls = web.unvisited();
    if unvisited_urls.len() == 0 {
        unvisited_urls = vec![Url::parse(BASE_URL)?];
    }
    for unvisited_url in unvisited_urls {
        println!("Trying unvisited url: {}", unvisited_url);
        web = visit_url(web, unvisited_url, DEPTH).await?;
    }

    // println!("Node count: {}", graph.node_count());
    // println!("Edge count: {}", graph.edge_count());

    let web_file = File::create("web.bincode")?;
    bincode::serialize_into(web_file, &web)?;

    web.to_dot("web.svg")?;
    Ok(())
}
