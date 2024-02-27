use petgraph::dot::{Config, Dot};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::str::FromStr;

use mime::Mime;
use petgraph::graph::{Graph, NodeIndex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

pub type GeminiGraph = Graph<Url, usize>;

#[derive(Default, Serialize, Deserialize)]
pub struct GeminiWeb {
    graph: GeminiGraph,
    pub visited: HashSet<Url>,
    url_node_ids: HashMap<Url, NodeIndex>,
    pub url_response: HashMap<Url, GeminiResponse>,
}

impl GeminiWeb {
    pub fn new() -> GeminiWeb {
        GeminiWeb {
            graph: Graph::new(),
            visited: HashSet::new(),
            url_node_ids: HashMap::new(),
            url_response: HashMap::new(),
        }
    }

    pub fn add_node(&mut self, url: &Url) -> NodeIndex {
        if let Some(index) = self.url_node_ids.get(url) {
            return *index;
        }
        let index = self.graph.add_node(url.clone());
        self.url_node_ids.insert(url.clone(), index);
        index
    }

    pub fn add_urls(&mut self, base_node_id: NodeIndex, urls: &[Url]) -> Vec<NodeIndex> {
        urls.iter()
            .map(|adjacent_url| {
                let node_id = self.add_node(adjacent_url);
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

    pub fn unvisited(&self) -> Vec<Url> {
        let registered_urls: HashSet<_> = self.url_node_ids.keys().cloned().collect();
        registered_urls.difference(&self.visited).cloned().collect()
    }

    pub fn to_dot(&self, path: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
        let path = path.as_ref();

        // Piping dot representation of graph to graphviz and writing the output to an image
        let mut dot_process = Command::new("dot")
            .arg(format!(
                "-T{}",
                path.extension()
                    .ok_or("path doesn't have extension")?
                    .to_str()
                    .unwrap()
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut dot_process_stdin = dot_process.stdin.take().expect("Failed to get stdin");
        let graph = self.graph.clone(); // TODO: could be slow
        std::thread::spawn(move || {
            let graph_dot = Dot::with_attr_getters(
                &graph,
                &[Config::EdgeNoLabel],
                &|_, _| String::new(),
                &|_, (_, url)| {
                    format!(
                        "label=\"{}{}\" group=\"{}\" fontname=\"monospace\"",
                        url.domain().unwrap(),
                        url.path(),
                        url.domain().unwrap(),
                    )
                },
            );
            dot_process_stdin
                .write_all(format!("{:?}", graph_dot).as_bytes())
                .expect("Counldn't write to stdin");
        });
        let output = dot_process.wait_with_output()?;
        let mut dot_file = File::create(path)?;
        dot_file.write_all(&output.stdout[..])?;
        Ok(())
    }
}

pub fn parse_body_urls(base_url: &Url, body: &str) -> Vec<Url> {
    body.lines()
        .filter_map(|line| {
            let line = line.strip_prefix("=>")?.trim().replace('\t', " ");
            let (adjacent_url, _label) = line.split_once(' ').unwrap_or((&line, ""));
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

fn serialize_mime<S: Serializer>(mime: &Mime, serializer: S) -> Result<S::Ok, S::Error> {
    mime.to_string().serialize(serializer)
}

fn deserialize_mime<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Mime, D::Error> {
    String::deserialize(deserializer)?
        .parse()
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum GeminiHeader {
    Input(String),
    #[serde(
        serialize_with = "serialize_mime",
        deserialize_with = "deserialize_mime"
    )]
    Success(Mime),
    Redirect(Url),
    TempFail(String),
    PermFail(String),
    Auth(String),
}

impl FromStr for GeminiHeader {
    type Err = Box<dyn Error>;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (status_code, rest) = s.split_once(' ').ok_or("no status code")?;
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiResponse {
    url: Url,
    pub header: GeminiHeader,
    pub body: GeminiText,
}

impl GeminiResponse {
    pub fn new(response: &str, url: &Url) -> Result<GeminiResponse, Box<dyn Error>> {
        let (header, body) = response
            .split_once("\r\n")
            .ok_or("Gemini response invalid format")?;
        let header: GeminiHeader = header.parse()?;
        let body = GeminiText::new(body, url)?;
        Ok(GeminiResponse {
            url: url.clone(),
            header,
            body,
        })
    }

    pub fn gemini_urls(&self) -> Vec<Url> {
        self.body
            .0
            .iter()
            .filter_map(|s| match s {
                GeminiTextStatement::Link(u, _) if u.scheme() == "gemini" => Some(u),
                _ => None,
            })
            .cloned()
            .collect()
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct GeminiText(Vec<GeminiTextStatement>);

#[derive(Debug, Serialize, Deserialize, Clone)]
enum GeminiTextStatement {
    Line(String),
    Link(Url, String),
    ListItem(String),
    Header(GeminiTextHeaderLevel, String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
enum GeminiTextHeaderLevel {
    L1,
    L2,
    L3,
}

impl GeminiText {
    fn new(body: &str, base_url: &Url) -> Result<GeminiText, Box<dyn Error>> {
        let url_parser = Url::options().base_url(Some(base_url));
        use GeminiTextHeaderLevel::*;
        use GeminiTextStatement::*;
        let mut in_pre = false;
        let mut text: GeminiText = Default::default();
        for line in body.lines() {
            let line = line.trim().to_string();
            if line == "```" {
                in_pre = !in_pre;
            }
            if in_pre {
                text.0.push(Line(line));
                continue;
            }
            let statement = match line.as_bytes() {
                [b'=', b'>', ..] => {
                    let line = line[2..].trim().replace('\t', " ");
                    let (url, label) = line.split_once(' ').unwrap_or((&line, ""));
                    Link(url_parser.parse(url.trim())?, label.trim().to_string())
                }
                [b'#', b'#', b'#', ..] => Header(L3, line[3..].trim().to_string()),
                [b'#', b'#', ..] => Header(L2, line[2..].trim().to_string()),
                [b'#', ..] => Header(L1, line[1..].trim().to_string()),
                [b'*', ..] => ListItem(line[1..].trim().to_string()),
                _ => Line(line),
            };
            text.0.push(statement);
        }
        Ok(text)
    }
}

use std::fmt;

impl fmt::Display for GeminiText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use GeminiTextHeaderLevel::*;
        use GeminiTextStatement::*;
        self.0
            .iter()
            .try_for_each(|statement| match statement {
                Line(s) => writeln!(f, "{}", s),
                Link(url, label) => writeln!(f, "=> {} ({})", url, label),
                ListItem(s) => writeln!(f, "* {}", s),
                Header(L1, s) => writeln!(f, "# {}", s),
                Header(L2, s) => writeln!(f, "## {}", s),
                Header(L3, s) => writeln!(f, "### {}", s),
            })
    }
}
