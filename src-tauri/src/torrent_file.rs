use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use sha1::{Digest, Sha1};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTorrent {
    pub info_hash: String,
    pub name: String,
    pub trackers: Vec<String>,
}

/// One bencoded dictionary entry: key, byte range of the value, parsed value.
type DictItem = (Vec<u8>, (usize, usize), Node);

#[derive(Debug)]
enum Node {
    Int(#[allow(dead_code)] i64),
    Bytes(Vec<u8>),
    List(Vec<((usize, usize), Node)>),
    Dict(Vec<DictItem>),
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }
    fn advance(&mut self) -> Result<u8> {
        let b = self.peek().ok_or_else(|| anyhow!("bencode: EOF inatteso"))?;
        self.pos += 1;
        Ok(b)
    }
    fn expect(&mut self, b: u8) -> Result<()> {
        let c = self.advance()?;
        if c != b {
            bail!("bencode: atteso {:?} a {}, trovato {:?}", b as char, self.pos - 1, c as char);
        }
        Ok(())
    }

    fn parse_node(&mut self) -> Result<((usize, usize), Node)> {
        let start = self.pos;
        let b = self
            .peek()
            .ok_or_else(|| anyhow!("bencode: EOF a {start}"))?;
        let node = match b {
            b'i' => {
                self.pos += 1;
                Node::Int(self.parse_int()?)
            }
            b'l' => {
                self.pos += 1;
                Node::List(self.parse_list_items()?)
            }
            b'd' => {
                self.pos += 1;
                Node::Dict(self.parse_dict_items()?)
            }
            b'0'..=b'9' => Node::Bytes(self.parse_bytes()?),
            _ => bail!("bencode: token non valido a {start}: 0x{:02x}", b),
        };
        Ok(((start, self.pos), node))
    }

    fn parse_int(&mut self) -> Result<i64> {
        let start = self.pos;
        while self.peek() != Some(b'e') {
            self.advance()?;
        }
        let s = std::str::from_utf8(&self.data[start..self.pos])
            .map_err(|e| anyhow!("bencode int utf8: {e}"))?;
        let v: i64 = s.parse().map_err(|e| anyhow!("bencode int parse: {e}"))?;
        self.expect(b'e')?;
        Ok(v)
    }

    fn parse_bytes(&mut self) -> Result<Vec<u8>> {
        let len_start = self.pos;
        while self.peek() != Some(b':') {
            self.advance()?;
        }
        let len: usize = std::str::from_utf8(&self.data[len_start..self.pos])
            .map_err(|e| anyhow!("bencode bytes len utf8: {e}"))?
            .parse()
            .map_err(|e| anyhow!("bencode bytes len parse: {e}"))?;
        self.expect(b':')?;
        if self.pos + len > self.data.len() {
            bail!("bencode bytes overflow");
        }
        let v = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(v)
    }

    fn parse_list_items(&mut self) -> Result<Vec<((usize, usize), Node)>> {
        let mut items = Vec::new();
        while self.peek() != Some(b'e') {
            items.push(self.parse_node()?);
        }
        self.expect(b'e')?;
        Ok(items)
    }

    fn parse_dict_items(&mut self) -> Result<Vec<DictItem>> {
        let mut items = Vec::new();
        while self.peek() != Some(b'e') {
            let key = self.parse_bytes()?;
            let (range, node) = self.parse_node()?;
            items.push((key, range, node));
        }
        self.expect(b'e')?;
        Ok(items)
    }
}

pub fn parse_torrent_bytes(bytes: &[u8]) -> Result<ParsedTorrent> {
    let mut p = Parser { data: bytes, pos: 0 };
    let (_, root) = p.parse_node()?;
    let Node::Dict(items) = root else {
        bail!("Torrent: root non è un dict bencode");
    };

    let mut info_hash: Option<String> = None;
    let mut name = String::new();
    let mut trackers: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push_tracker = |s: String, trackers: &mut Vec<String>, seen: &mut HashSet<String>| {
        if !s.is_empty() && seen.insert(s.clone()) {
            trackers.push(s);
        }
    };

    for (key, range, node) in &items {
        match key.as_slice() {
            b"info" => {
                let mut h = Sha1::new();
                h.update(&bytes[range.0..range.1]);
                let hash = h.finalize();
                info_hash = Some(hash.iter().map(|b| format!("{:02x}", b)).collect());
                if let Node::Dict(info_items) = node {
                    if let Some((_, _, Node::Bytes(nb))) = info_items
                        .iter()
                        .find(|(k, _, _)| k.as_slice() == b"name")
                    {
                        name = String::from_utf8_lossy(nb).into_owned();
                    }
                }
            }
            b"announce" => {
                if let Node::Bytes(b) = node {
                    push_tracker(
                        String::from_utf8_lossy(b).into_owned(),
                        &mut trackers,
                        &mut seen,
                    );
                }
            }
            b"announce-list" => {
                if let Node::List(tiers) = node {
                    for (_, tier) in tiers {
                        if let Node::List(items_inner) = tier {
                            for (_, item) in items_inner {
                                if let Node::Bytes(b) = item {
                                    push_tracker(
                                        String::from_utf8_lossy(b).into_owned(),
                                        &mut trackers,
                                        &mut seen,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let info_hash = info_hash.ok_or_else(|| anyhow!("Torrent senza dict 'info'"))?;
    tracing::info!(
        "[torrent_file] parsed info_hash={} name={:?} trackers={}",
        info_hash,
        name,
        trackers.len()
    );
    Ok(ParsedTorrent { info_hash, name, trackers })
}
