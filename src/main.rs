// use tokio;
// use tokio::net::TcpStream;
// use tokio::io::AsyncWriteExt;
// use tokio::io::AsyncReadExt;
// use native_tls::TlsConnector;
// use futures::stream::FuturesUnordered;
// use futures::prelude::*;

use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use petgraph::dot::{Config, Dot};
use petgraph::graph::{Graph, NodeIndex};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::net::TcpStream;
use std::process::{Command, Stdio};
use url::Url;

type GeminiGraph = Graph<String, usize>;

fn graph_add_node(
    graph: &mut GeminiGraph,
    url_node_ids: &mut HashMap<String, NodeIndex>,
    url: &Url,
) -> NodeIndex {
    let key = format!("{}{}", url.domain().unwrap(), url.path());
    if let Some(index) = url_node_ids.get(&key) {
        return *index;
    }
    let index = graph.add_node(key.clone());
    url_node_ids.insert(key, index);
    index
}

fn visit_url_recursion(
    base_url: Url,
    base_node_id: NodeIndex,
    graph: &mut GeminiGraph,
    visited: &mut HashSet<Url>,
    url_node_ids: &mut HashMap<String, NodeIndex>,
    depth: usize,
) -> Result<(), Box<dyn Error>> {
    if depth == 0 || visited.contains(&base_url) {
        return Ok(());
    }
    eprintln!("Visiting {}", base_url.to_string());
    visited.insert(base_url.clone());
    // Setup SSL
    let mut connector_builder = SslConnector::builder(SslMethod::tls())?;
    connector_builder.set_verify(SslVerifyMode::NONE);
    let connector = connector_builder.build();
    // Connect to base url and query the gemini page
    let base_domain = base_url.domain().unwrap();
    let base_domain_port = base_domain.to_owned() + ":1965";
    let stream = TcpStream::connect(base_domain_port)?;
    // let cx = TlsConnector::builder().build()?;
    // let cx = tokio_native_tls::TlsConnector::from(cx);
    // let mut stream = cx.connect(base_domain, stream).await?;
    let mut stream = connector.connect(base_domain, stream)?;
    stream.write_all((base_url.to_string() + "\r\n").as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?; // TODO: check if response contains error
                                           // Parse links in the response

    response
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
            let node_id = graph_add_node(graph, url_node_ids, &adjacent_url);

            let edge_weight = match graph.find_edge(base_node_id, node_id) {
                Some(e) => graph.edge_weight(e).unwrap() + 1,
                None => 1,
            };
            graph.update_edge(base_node_id, node_id, edge_weight);
            visit_url_recursion(
                adjacent_url,
                node_id,
                graph,
                visited,
                url_node_ids,
                depth - 1,
            )
        })
        .collect::<Result<(), _>>()
}

fn visit_url(base_url: Url, depth: usize) -> Result<GeminiGraph, Box<dyn Error>> {
    let mut graph = Graph::new();
    let mut visited = HashSet::new();
    let mut url_node_ids = HashMap::new();
    let base_node_id = graph_add_node(&mut graph, &mut url_node_ids, &base_url);
    visit_url_recursion(
        base_url,
        base_node_id,
        &mut graph,
        &mut visited,
        &mut url_node_ids,
        depth,
    )?;
    Ok(graph)
}

const BASE_URL: &str = "gemini://makeworld.space:1965/amfora-wiki/";
const DEPTH: usize = 3;

fn main() -> Result<(), Box<dyn Error>> {
    let base_url = Url::parse(BASE_URL)?;
    let graph = visit_url(base_url, DEPTH)?;
    // FIXME: too many nodes compared to number of visited pages (~10 pages visited but got ~100
    // nodes), more than one key is created for each pages during the recursion.
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
