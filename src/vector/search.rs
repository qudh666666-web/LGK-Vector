use xmltree::{Element, XMLNode};

pub fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.children.iter().find_map(|node| match node {
        XMLNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

pub fn child_text(element: &Element, name: &str) -> Option<String> {
    child(element, name)
        .and_then(Element::get_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn descendants<'a>(element: &'a Element, name: &str, output: &mut Vec<&'a Element>) {
    if element.name == name {
        output.push(element);
    }
    for node in &element.children {
        if let XMLNode::Element(child) = node {
            descendants(child, name, output);
        }
    }
}

pub fn first_descendant_text(element: &Element, names: &[&str]) -> Option<String> {
    if names.iter().any(|name| element.name == *name) {
        return element
            .get_text()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    for node in &element.children {
        if let XMLNode::Element(child) = node {
            if let Some(value) = first_descendant_text(child, names) {
                return Some(value);
            }
        }
    }
    None
}

pub fn is_definition(element: &Element) -> bool {
    element.name == "ECUC-PARAM-CONF-CONTAINER-DEF"
        || element.name == "ECUC-CHOICE-CONTAINER-DEF"
        || element.name == "ECUC-FUNCTION-NAME-DEF"
        || element.name.ends_with("-PARAM-DEF")
        || element.name.ends_with("-REFERENCE-DEF")
}
