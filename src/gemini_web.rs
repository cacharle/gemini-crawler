use petgraph::dot::{Config, Dot};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::process::{Command, Stdio};
use std::str::FromStr;

use petgraph::graph::{Graph, NodeIndex};
use url::Url;
use mime::Mime;
// use serde::{Serialize, Deserialize};

pub type GeminiGraph = Graph<String, usize>;

// #[derive(Serialize, Deserialize)]
#[derive(Default)]
pub struct GeminiWeb {
    graph: GeminiGraph,
    visited: HashSet<Url>,
    url_node_ids: HashMap<String, NodeIndex>,
}

impl GeminiWeb {
    pub fn new() -> GeminiWeb {
        GeminiWeb {
            graph: Graph::new(),
            visited: HashSet::new(),
            url_node_ids: HashMap::new(),
        }
    }

    pub fn add_node(&mut self, url: &Url) -> NodeIndex {
        let key = format!("{}{}", url.domain().unwrap(), url.path());
        if let Some(index) = self.url_node_ids.get(&key) {
            return *index;
        }
        let index = self.graph.add_node(key.clone());
        self.url_node_ids.insert(key, index);
        index
    }

    pub fn add_urls(&mut self, base_node_id: NodeIndex, urls: &Vec<Url>) -> Vec<NodeIndex> {
        urls.iter()
            .map(|adjacent_url| {
                let node_id = self.add_node(&adjacent_url);
                let edge_weight = match self.graph.find_edge(base_node_id, node_id) {
                    Some(e) => self.graph.edge_weight(e).unwrap() + 1,
                    None => 1,
                };
                self.graph.update_edge(base_node_id, node_id, edge_weight);
                node_id
            })
            .collect()
    }

    pub fn try_visit(&mut self, url: &Url) -> bool {
        !self.visited.insert(url.clone())
    }

    pub fn to_dot(&self, file_type: &str) -> Result<(), Box<dyn Error>> {
        // Piping dot representation of graph to graphviz and writing the output to an image
        let mut dot_process = Command::new("dot")
            .arg(format!("-T{file_type}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut dot_process_stdin = dot_process.stdin.take().expect("Failed to get stdin");
        let graph = self.graph.clone(); // TODO: could be slow
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
}

pub fn parse_body_urls(base_url: &Url, body: &str) -> Vec<Url> {
    body.lines()
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
        .collect()
}

#[derive(Debug)]
pub enum GeminiHeader {
    Input(String),
    Success(Mime), // string is mime type
    Redirect(Url),
    TempFail(String),
    PermFail(String),
    Auth(String),
}

impl FromStr for GeminiHeader {
    type Err = Box<dyn Error>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (status_code, rest) = s.split_once(" ").ok_or("no status code")?;
        let status_code: u64 = status_code.parse()?;
        let rest = rest.to_string();
        use GeminiHeader::*;
        let header = match status_code {
            10..=19 => Input(rest),
            20..=29 => Success(rest.parse()?),
            30..=39 => Redirect(Url::parse(&rest)?),
            40..=49 => TempFail(rest),
            50..=59 => PermFail(rest),
            60..=69 => Auth(rest),
            _ => return Err("invalid status code".into()),
        };
        Ok(header)
    }
}
