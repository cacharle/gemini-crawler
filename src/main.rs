use std::cell::RefCell;
use std::error::Error;
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

use gemini_web::GeminiWeb;

const TIMEOUT: Duration = Duration::from_secs(2);

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
    let base_domain = base_url.domain().unwrap();
    let base_domain_port = base_domain.to_owned() + ":1965";
    // Setup SSL
    let stream = timeout(TIMEOUT, TcpStream::connect(base_domain_port)).await??;
    let cx = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()?;
    let cx = tokio_native_tls::TlsConnector::from(cx);
    // Connect to base url and query the gemini page
    let mut stream = timeout(TIMEOUT, cx.connect(base_domain, stream)).await??;
    timeout(
        TIMEOUT,
        stream.write_all((base_url.to_string() + "\r\n").as_bytes()),
    )
    .await??;
    let mut response = String::new();
    // TODO: check if response contains error
    timeout(TIMEOUT, stream.read_to_string(&mut response)).await??;
    let (header, body) = response
        .split_once("\r\n")
        .ok_or("Gemini response invalid format")?;
    if !header.starts_with("20") {
        println!("{}", header);
    }

    let urls = gemini_web::parse_body_urls(&base_url, body);
    let node_ids = web.borrow_mut().add_urls(base_node_id, &urls);

    let mut fs = urls
        .iter()
        .zip(node_ids)
        .map(|(url, node_id)| visit_url_recursion(url.clone(), node_id, web.clone(), depth - 1))
        .collect::<FuturesUnordered<_>>();
    while let Some(_res) = fs.next().await {}
    Ok(())
}

async fn visit_url(base_url: Url, depth: usize) -> Result<GeminiWeb, Box<dyn Error>> {
    // let graph = match File::open("graph.bincode") {
    //     Ok(reader) => bincode::deserialize_from(reader)?,
    //     _ => Graph::new(),
    // };
    let web = Rc::new(RefCell::new(GeminiWeb::new()));
    let base_node_id = web.borrow_mut().add_node(&base_url);
    visit_url_recursion(base_url, base_node_id, web.clone(), depth).await?;
    Ok(web.take()) // FIXME: understand why into_inner() doesn't work here
}

const BASE_URL: &str = "gemini://makeworld.space:1965/amfora-wiki/";
const DEPTH: usize = 2;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let base_url = Url::parse(BASE_URL)?;
    let web = visit_url(base_url, DEPTH).await?;

    // println!("Node count: {}", graph.node_count());
    // println!("Edge count: {}", graph.edge_count());

    // let graph_file = File::create("graph.bincode")?;
    // bincode::serialize_into(graph_file, &graph)?;

    web.to_dot("svg")?;
    Ok(())
}
