//! Strict, bounded XML; no DTD or entity resolver.
pub(crate) const MAX_BODY: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Invalid,
    Secrets,
    Vault,
    Certificate,
    Refused,
    Locked,
    Unsupported,
    Transport,
    Identity,
    Budget,
}
pub(crate) fn parse(body: &[u8]) -> Result<roxmltree::Document<'_>, Failure> {
    if body.len() > MAX_BODY {
        return Err(Failure::Invalid);
    }
    let text = std::str::from_utf8(body).map_err(|_| Failure::Invalid)?;
    // roxmltree permits an empty DTD even with allow_dtd=false. The contract
    // refuses that form too; no external entity resolver is ever installed.
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
    let mut elements = 0;
    for node in doc.descendants() {
        if node.is_element() {
            elements += 1;
            if elements > 10_000
                || node.ancestors().filter(roxmltree::Node::is_element).count() > 16
            {
                return Err(Failure::Invalid);
            }
            let length = node
                .children()
                .filter(roxmltree::Node::is_text)
                .map(|text| text.text().unwrap_or("").len())
                .sum::<usize>();
            if length > 4096 || node.attributes().any(|a| a.value().len() > 4096) {
                return Err(Failure::Invalid);
            }
        }
    }
    Ok(doc)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_xml_and_namespaces_parse_without_repair() {
        assert!(parse(b"<s:Envelope xmlns:s='http://schemas.xmlsoap.org/soap/envelope/'><s:Body/></s:Envelope>").is_ok());
    }
    #[test]
    fn rejects_unsafe_xml_and_all_resource_limits() {
        for body in [
            "<!DOCTYPE a [<!ENTITY ext SYSTEM 'http://192.168.1.9/'>]><a>&ext;</a>",
            "<!DOCTYPE a><a/>",
            "<a>&undeclared;</a>",
            "<a>",
            "<a/><a/>",
            "<a>\u{1}</a>",
        ] {
            assert!(parse(body.as_bytes()).is_err());
        }
        assert!(parse(&vec![b' '; MAX_BODY + 1]).is_err());
        assert!(parse(format!("{}{}", "<a>".repeat(17), "</a>".repeat(17)).as_bytes()).is_err());
        assert!(parse(format!("<a>{}</a>", "x".repeat(4097)).as_bytes()).is_err());
        assert!(parse(format!("<a>{}</a>", "<b/>".repeat(10_000)).as_bytes()).is_err());
    }
}
