use tokio;
use tokio::net::TcpStream;
use tokio::io::AsyncWriteExt;
use tokio::io::AsyncReadExt;
use tokio::time::timeout;

// use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

use native_tls::TlsConnector;

use petgraph::dot::{Config, Dot};
use petgraph::graph::{Graph, NodeIndex};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
// use std::net::TcpStream;
use std::process::{Command, Stdio};
use url::Url;

type GeminiGraph = Graph<String, usize>;

fn graph_add_node(
    graph: Rc<RefCell<GeminiGraph>>,
    url_node_ids: Rc<RefCell<HashMap<String, NodeIndex>>>,
    url: &Url,
) -> NodeIndex {
    let key = format!("{}{}", url.domain().unwrap(), url.path());
    if let Some(index) = url_node_ids.borrow().get(&key) {
        return *index;
    }
    let index = graph.borrow_mut().add_node(key.clone());
    url_node_ids.borrow_mut().insert(key, index);
    index
}

use std::cell::RefCell;
use std::rc::Rc;
use async_recursion::async_recursion;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

#[async_recursion(?Send)]
async fn visit_url_recursion(
    base_url: Url,
    base_node_id: NodeIndex,
    graph: Rc<RefCell<GeminiGraph>>,
    visited: Rc<RefCell<HashSet<Url>>>,
    url_node_ids: Rc<RefCell<HashMap<String, NodeIndex>>>,
    depth: usize,
) -> Result<(), Box<dyn Error>> {
    if depth == 0 || visited.borrow().contains(&base_url) {
        return Ok(());
    }
    eprintln!("Visiting {}", base_url.to_string());
    visited.borrow_mut().insert(base_url.clone());
    // Setup SSL

    // let mut connector_builder = SslConnector::builder(SslMethod::tls())?;
    // connector_builder.set_verify(SslVerifyMode::NONE);
    // let connector = connector_builder.build();

    // Connect to base url and query the gemini page
    let base_domain = base_url.domain().unwrap();
    let base_domain_port = base_domain.to_owned() + ":1965";
    let stream = timeout(TIMEOUT, TcpStream::connect(base_domain_port)).await??;
    let cx = TlsConnector::builder().danger_accept_invalid_certs(true).build()?;
    let cx = tokio_native_tls::TlsConnector::from(cx);
    let mut stream = timeout(TIMEOUT, cx.connect(base_domain, stream)).await??;
    // let mut stream = connector.connect(base_domain, stream)?;
    timeout(TIMEOUT, stream.write_all((base_url.to_string() + "\r\n").as_bytes())).await??;
    let mut response = String::new();
    // TODO: check if response contains error
    timeout(TIMEOUT, stream.read_to_string(&mut response)).await??;

    use futures::stream::FuturesUnordered;
    use futures::prelude::*;

    // Parse links in response
    let mut fs = response
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("=>")?.trim().replace("\t", " ");
            let (adjacent_url, _label) = line.split_once(" ").unwrap_or((&line, ""));
            match Url::parse(adjacent_url) {
                Ok(url) => (url.scheme() == "gemini").then_some(url),
                Err(url::ParseError::RelativeUrlWithoutBase) => {
                    Some(base_url.join(adjacent_url).unwrap())
                }
                Err(e) => {
                    eprintln!("Error parsing '{adjacent_url}' url: {e}");
                    None
                }
            }
        })
        .map(|adjacent_url| {
            let node_id = graph_add_node(graph.clone(), url_node_ids.clone(), &adjacent_url);

            let edge_weight = match graph.borrow().find_edge(base_node_id, node_id) {
                Some(e) => graph.borrow().edge_weight(e).unwrap() + 1,
                None => 1,
            };
            graph.borrow_mut().update_edge(base_node_id, node_id, edge_weight);
            visit_url_recursion(
                adjacent_url,
                node_id,
                graph.clone(),
                visited.clone(),
                url_node_ids.clone(),
                depth - 1,
            )
        })
        // .collect::<Result<(), _>>()
        .collect::<FuturesUnordered<_>>();
    while let Some(_res) = fs.next().await {
    }
    Ok(())
}

async fn visit_url(base_url: Url, depth: usize) -> Result<Rc<RefCell<GeminiGraph>>, Box<dyn Error>> {
    let graph = Rc::new(RefCell::new(Graph::new()));
    let visited = Rc::new(RefCell::new(HashSet::new()));
    let url_node_ids = Rc::new(RefCell::new(HashMap::new()));
    let base_node_id = graph_add_node(graph.clone(), url_node_ids.clone(), &base_url);
    visit_url_recursion(
        base_url,
        base_node_id,
        graph.clone(),
        visited,
        url_node_ids,
        depth,
    ).await?;
    Ok(graph)
}

const BASE_URL: &str = "gemini://makeworld.space:1965/amfora-wiki/";
const DEPTH: usize = 5;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let base_url = Url::parse(BASE_URL)?;
    let graph = visit_url(base_url, DEPTH).await?;
    let graph = graph.take(); // FIXME: understand why into_inner() doesn't work here
    println!("Node count: {}", graph.node_count());
    println!("Edge count: {}", graph.edge_count());
    // Piping dot representation of graph to graphviz and writing the output to an image
    let mut dot_process = Command::new("dot")
        .arg("-Tsvg")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut dot_process_stdin = dot_process.stdin.take().expect("Failed to get stdin");
    std::thread::spawn(move || {
        let graph_dot = Dot::with_config(&graph, &[Config::EdgeNoLabel]);
        dot_process_stdin
            .write_all(format!("{:?}", graph_dot).as_bytes())
            .expect("Counldn't write to stdin");
    });
    let output = dot_process.wait_with_output()?;
    let mut dot_file = File::create("graph.svg")?;
    dot_file.write_all(&output.stdout[..])?;
    Ok(())
}
