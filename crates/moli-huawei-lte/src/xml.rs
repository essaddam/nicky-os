//! A bounded, flat HiLink document. No DTD, entity resolver or repair.
use crate::Failure;
use std::collections::BTreeMap;
pub(crate) const MAX_BODY: usize = 1024 * 1024;
#[derive(Debug, Default)]
pub(crate) struct Fields {
    pub values: BTreeMap<String, String>,
    pub ok: bool,
}
pub(crate) fn parse(body: &[u8]) -> Result<Fields, Failure> {
    if body.len() > MAX_BODY {
        return Err(Failure::Invalid);
    }
    let text = std::str::from_utf8(body).map_err(|_| Failure::Invalid)?;
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(Failure::Invalid);
    }
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 30_001,
            entity_resolver: None,
        },
    )
    .map_err(|_| Failure::Invalid)?;
    let mut count = 0;
    for node in doc.descendants().filter(roxmltree::Node::is_element) {
        count += 1;
        if count > 10_000
            || node.ancestors().filter(roxmltree::Node::is_element).count() > 16
            || node
                .children()
                .filter(roxmltree::Node::is_text)
                .map(|n| n.text().unwrap_or("").len())
                .sum::<usize>()
                > 4096
            || node.attributes().any(|a| a.value().len() > 4096)
        {
            return Err(Failure::Invalid);
        }
    }
    let root = doc.root_element();
    if root.tag_name().namespace().is_some()
        || !matches!(root.tag_name().name(), "response" | "error")
    {
        return Err(Failure::Invalid);
    }
    let mut fields = Fields::default();
    for node in root.children().filter(roxmltree::Node::is_element) {
        if node.tag_name().namespace().is_some() || node.children().any(|n| n.is_element()) {
            return Err(Failure::Invalid);
        }
        let value = node
            .children()
            .filter(roxmltree::Node::is_text)
            .map(|n| n.text().unwrap_or(""))
            .collect::<String>();
        if fields
            .values
            .insert(node.tag_name().name().to_owned(), value.trim().to_owned())
            .is_some()
        {
            return Err(Failure::Invalid);
        }
    }
    if root.tag_name().name() == "error" {
        let code = fields
            .values
            .get("code")
            .and_then(|s| s.parse::<u32>().ok())
            .ok_or(Failure::Invalid)?;
        return Err(match code {
            100_002 => Failure::Unsupported,
            100_003 | 125_001..=125_003 => Failure::Expired,
            100_004 => Failure::Transport,
            108_001..=108_007 | 115_002 => Failure::Refused,
            _ => Failure::Invalid,
        });
    }
    let text = root
        .children()
        .filter(roxmltree::Node::is_text)
        .map(|n| n.text().unwrap_or(""))
        .collect::<String>();
    if !text.trim().is_empty() {
        if text.trim() != "OK" || !fields.values.is_empty() {
            return Err(Failure::Invalid);
        }
        fields.ok = true;
    }
    Ok(fields)
}
