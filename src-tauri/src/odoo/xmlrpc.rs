use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use std::collections::HashMap;
use std::io::Cursor;

use crate::error::{AppError, AppResult};

/// Map `std::io::Error` (from `Writer::write_event`) to our `AppError`.
fn io_err(e: std::io::Error) -> AppError {
    AppError::Odoo(format!("XML write error: {e}"))
}

// ---------------------------------------------------------------------------
// XmlRpcValue
// ---------------------------------------------------------------------------

/// Represents every value type in the XML-RPC protocol, plus `<nil/>` which
/// Odoo uses for Python `None`.
#[derive(Debug, Clone, PartialEq)]
pub enum XmlRpcValue {
    Int(i64),
    Bool(bool),
    Double(f64),
    String(String),
    Array(Vec<XmlRpcValue>),
    Struct(HashMap<String, XmlRpcValue>),
    Nil,
}

impl XmlRpcValue {
    // -- convenience accessors ------------------------------------------------

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            XmlRpcValue::Int(v) => Some(*v),
            XmlRpcValue::Double(v) => Some(*v as i64),
            XmlRpcValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            XmlRpcValue::Double(v) => Some(*v),
            XmlRpcValue::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            XmlRpcValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            XmlRpcValue::Bool(b) => Some(*b),
            XmlRpcValue::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_array(&self) -> Option<&Vec<XmlRpcValue>> {
        match self {
            XmlRpcValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&HashMap<String, XmlRpcValue>> {
        match self {
            XmlRpcValue::Struct(m) => Some(m),
            _ => None,
        }
    }

    /// Odoo quirk: a many2one field is either `[id, "name"]` or `false`.
    /// This helper returns `None` for both `Nil` and `Bool(false)`.
    pub fn as_many2one(&self) -> Option<(i64, String)> {
        match self {
            XmlRpcValue::Bool(false) | XmlRpcValue::Nil => None,
            XmlRpcValue::Array(arr) if arr.len() == 2 => {
                let id = arr[0].as_i64()?;
                let name = arr[1].as_str()?.to_string();
                Some((id, name))
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Serialization — build <methodCall> XML
// ---------------------------------------------------------------------------

fn write_value(writer: &mut Writer<Cursor<Vec<u8>>>, val: &XmlRpcValue) -> AppResult<()> {
    writer.write_event(Event::Start(BytesStart::new("value"))).map_err(io_err)?;

    match val {
        XmlRpcValue::Int(i) => {
            writer.write_event(Event::Start(BytesStart::new("int"))).map_err(io_err)?;
            writer.write_event(Event::Text(BytesText::new(&i.to_string()))).map_err(io_err)?;
            writer.write_event(Event::End(BytesEnd::new("int"))).map_err(io_err)?;
        }
        XmlRpcValue::Bool(b) => {
            writer.write_event(Event::Start(BytesStart::new("boolean"))).map_err(io_err)?;
            writer.write_event(Event::Text(BytesText::new(if *b { "1" } else { "0" }))).map_err(io_err)?;
            writer.write_event(Event::End(BytesEnd::new("boolean"))).map_err(io_err)?;
        }
        XmlRpcValue::Double(d) => {
            writer.write_event(Event::Start(BytesStart::new("double"))).map_err(io_err)?;
            writer.write_event(Event::Text(BytesText::new(&d.to_string()))).map_err(io_err)?;
            writer.write_event(Event::End(BytesEnd::new("double"))).map_err(io_err)?;
        }
        XmlRpcValue::String(s) => {
            writer.write_event(Event::Start(BytesStart::new("string"))).map_err(io_err)?;
            writer.write_event(Event::Text(BytesText::new(s))).map_err(io_err)?;
            writer.write_event(Event::End(BytesEnd::new("string"))).map_err(io_err)?;
        }
        XmlRpcValue::Array(arr) => {
            writer.write_event(Event::Start(BytesStart::new("array"))).map_err(io_err)?;
            writer.write_event(Event::Start(BytesStart::new("data"))).map_err(io_err)?;
            for item in arr {
                write_value(writer, item)?;
            }
            writer.write_event(Event::End(BytesEnd::new("data"))).map_err(io_err)?;
            writer.write_event(Event::End(BytesEnd::new("array"))).map_err(io_err)?;
        }
        XmlRpcValue::Struct(map) => {
            writer.write_event(Event::Start(BytesStart::new("struct"))).map_err(io_err)?;
            for (key, v) in map {
                writer.write_event(Event::Start(BytesStart::new("member"))).map_err(io_err)?;
                writer.write_event(Event::Start(BytesStart::new("name"))).map_err(io_err)?;
                writer.write_event(Event::Text(BytesText::new(key))).map_err(io_err)?;
                writer.write_event(Event::End(BytesEnd::new("name"))).map_err(io_err)?;
                write_value(writer, v)?;
                writer.write_event(Event::End(BytesEnd::new("member"))).map_err(io_err)?;
            }
            writer.write_event(Event::End(BytesEnd::new("struct"))).map_err(io_err)?;
        }
        XmlRpcValue::Nil => {
            writer.write_event(Event::Empty(BytesStart::new("nil"))).map_err(io_err)?;
        }
    }

    writer.write_event(Event::End(BytesEnd::new("value"))).map_err(io_err)?;
    Ok(())
}

fn build_method_call(method: &str, params: &[XmlRpcValue]) -> AppResult<String> {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer.write_event(Event::Start(BytesStart::new("methodCall"))).map_err(io_err)?;

    writer.write_event(Event::Start(BytesStart::new("methodName"))).map_err(io_err)?;
    writer.write_event(Event::Text(BytesText::new(method))).map_err(io_err)?;
    writer.write_event(Event::End(BytesEnd::new("methodName"))).map_err(io_err)?;

    writer.write_event(Event::Start(BytesStart::new("params"))).map_err(io_err)?;
    for p in params {
        writer.write_event(Event::Start(BytesStart::new("param"))).map_err(io_err)?;
        write_value(&mut writer, p)?;
        writer.write_event(Event::End(BytesEnd::new("param"))).map_err(io_err)?;
    }
    writer.write_event(Event::End(BytesEnd::new("params"))).map_err(io_err)?;

    writer.write_event(Event::End(BytesEnd::new("methodCall"))).map_err(io_err)?;

    let bytes = writer.into_inner().into_inner();
    let xml = String::from_utf8(bytes).map_err(|e| AppError::Odoo(e.to_string()))?;
    Ok(xml)
}

// ---------------------------------------------------------------------------
// Deserialization — parse <methodResponse> XML
// ---------------------------------------------------------------------------

/// Stateful parser that walks through the quick-xml event stream.
struct XmlRpcParser<'a> {
    reader: Reader<&'a [u8]>,
    buf: Vec<u8>,
}

impl<'a> XmlRpcParser<'a> {
    fn new(xml: &'a str) -> Self {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        Self {
            reader,
            buf: Vec::new(),
        }
    }

    /// Read the next event, returning it as owned.
    fn next_event(&mut self) -> AppResult<Event<'static>> {
        let event = self.reader.read_event_into(&mut self.buf)?;
        Ok(event.into_owned())
    }

    /// Read text content until the matching end tag.
    fn read_text(&mut self) -> AppResult<String> {
        let mut text = String::new();
        loop {
            match self.next_event()? {
                Event::Text(e) => {
                    let decoded = e.decode().map_err(|e| AppError::Odoo(e.to_string()))?;
                    let unescaped = quick_xml::escape::unescape(&decoded)
                        .map_err(|e| AppError::Odoo(e.to_string()))?;
                    text.push_str(&unescaped);
                }
                Event::End(_) => return Ok(text),
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF reading text".into())),
                _ => continue,
            }
        }
    }

    /// Parse a `<value>...</value>` element. Caller has already consumed `<value>`.
    fn parse_value(&mut self) -> AppResult<XmlRpcValue> {
        // Inside <value> we expect a type tag, or bare text (treated as string), or </value> for empty string
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    let val = self.parse_typed_value(&tag)?;
                    // After parsing the typed value, consume remaining content until </value>
                    // (the typed element's end tag was already consumed by parse_typed_value)
                    return Ok(val);
                }
                Event::Empty(e) => {
                    let tag = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if tag == "nil" {
                        return Ok(XmlRpcValue::Nil);
                    }
                    return Ok(XmlRpcValue::String(String::new()));
                }
                Event::Text(e) => {
                    // Bare text inside <value>text</value> — treated as string
                    let decoded = e.decode().map_err(|e| AppError::Odoo(e.to_string()))?;
                    let text = quick_xml::escape::unescape(&decoded)
                        .map_err(|e| AppError::Odoo(e.to_string()))?
                        .to_string();
                    return Ok(XmlRpcValue::String(text));
                }
                Event::End(_) => {
                    // <value></value> — empty string
                    return Ok(XmlRpcValue::String(String::new()));
                }
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in value".into())),
                _ => continue,
            }
        }
    }

    /// Parse a typed value after its opening tag has been consumed.
    /// Consumes up to and including the closing tag of this typed element.
    fn parse_typed_value(&mut self, tag: &str) -> AppResult<XmlRpcValue> {
        match tag {
            "int" | "i4" | "i8" => {
                let t = self.read_text()?;
                let n: i64 = t.trim().parse().map_err(|e: std::num::ParseIntError| {
                    AppError::Odoo(format!("Bad int: {e}"))
                })?;
                Ok(XmlRpcValue::Int(n))
            }
            "boolean" => {
                let t = self.read_text()?;
                Ok(XmlRpcValue::Bool(t.trim() == "1" || t.trim() == "true"))
            }
            "double" => {
                let t = self.read_text()?;
                let n: f64 = t.trim().parse().map_err(|e: std::num::ParseFloatError| {
                    AppError::Odoo(format!("Bad double: {e}"))
                })?;
                Ok(XmlRpcValue::Double(n))
            }
            "string" => {
                let t = self.read_text()?;
                Ok(XmlRpcValue::String(t))
            }
            "nil" => {
                // consume </nil>
                self.read_text()?;
                Ok(XmlRpcValue::Nil)
            }
            "array" => self.parse_array(),
            "struct" => {
                let map = self.parse_struct()?;
                Ok(XmlRpcValue::Struct(map))
            }
            other => {
                // Unknown tag — skip content and treat as string
                let t = self.read_text()?;
                log::warn!("Unknown XML-RPC type tag <{other}>, treating as string");
                Ok(XmlRpcValue::String(t))
            }
        }
    }

    /// Parse array contents. Caller consumed `<array>`. Consumes through `</array>`.
    fn parse_array(&mut self) -> AppResult<XmlRpcValue> {
        let mut items = Vec::new();
        // We're inside <array>, expect <data>...</data></array>
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    match name.as_str() {
                        "data" => {
                            // Now read <value> elements until </data>
                            items = self.parse_data_values()?;
                        }
                        "value" => {
                            // Some XML-RPC impls skip <data> wrapper
                            items.push(self.parse_value()?);
                        }
                        _ => {}
                    }
                }
                Event::End(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "array" {
                        return Ok(XmlRpcValue::Array(items));
                    }
                }
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in array".into())),
                _ => continue,
            }
        }
    }

    /// Parse <value> elements inside <data>...</data>. Consumes through </data>.
    fn parse_data_values(&mut self) -> AppResult<Vec<XmlRpcValue>> {
        let mut items = Vec::new();
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "value" {
                        items.push(self.parse_value()?);
                    }
                }
                Event::End(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "data" {
                        return Ok(items);
                    }
                }
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in data".into())),
                _ => continue,
            }
        }
    }

    /// Parse struct contents. Caller consumed `<struct>`. Consumes through `</struct>`.
    fn parse_struct(&mut self) -> AppResult<HashMap<String, XmlRpcValue>> {
        let mut map = HashMap::new();
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "member" {
                        let (key, val) = self.parse_member()?;
                        map.insert(key, val);
                    }
                }
                Event::End(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "struct" {
                        return Ok(map);
                    }
                }
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in struct".into())),
                _ => continue,
            }
        }
    }

    /// Parse a single <member>. Caller consumed `<member>`. Consumes through `</member>`.
    fn parse_member(&mut self) -> AppResult<(String, XmlRpcValue)> {
        let mut key = String::new();
        let mut val = XmlRpcValue::Nil;
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    match name.as_str() {
                        "name" => { key = self.read_text()?; }
                        "value" => { val = self.parse_value()?; }
                        _ => {}
                    }
                }
                Event::End(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "member" {
                        return Ok((key, val));
                    }
                }
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in member".into())),
                _ => continue,
            }
        }
    }

    /// Parse the entire `<methodResponse>`.
    fn parse_response(&mut self) -> AppResult<XmlRpcValue> {
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "methodResponse" {
                        return self.parse_method_response_body();
                    }
                }
                Event::Eof => return Err(AppError::Odoo("No methodResponse found".into())),
                _ => continue,
            }
        }
    }

    fn parse_method_response_body(&mut self) -> AppResult<XmlRpcValue> {
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    match name.as_str() {
                        "params" => return self.parse_params_value(),
                        "fault" => return self.parse_fault_value(),
                        _ => continue,
                    }
                }
                Event::End(_) => return Err(AppError::Odoo("Empty methodResponse".into())),
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in methodResponse".into())),
                _ => continue,
            }
        }
    }

    /// Extract the value from <params><param><value>...</value></param></params>
    fn parse_params_value(&mut self) -> AppResult<XmlRpcValue> {
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "value" {
                        return self.parse_value();
                    }
                }
                Event::End(_) => return Err(AppError::Odoo("No value in params".into())),
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in params".into())),
                _ => continue,
            }
        }
    }

    fn parse_fault_value(&mut self) -> AppResult<XmlRpcValue> {
        loop {
            match self.next_event()? {
                Event::Start(e) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                    if name == "value" {
                        let val = self.parse_value()?;
                        let msg = match &val {
                            XmlRpcValue::Struct(m) => {
                                let fault_string = m
                                    .get("faultString")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown fault");
                                let fault_code = m
                                    .get("faultCode")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0);
                                format!("XML-RPC fault {fault_code}: {fault_string}")
                            }
                            _ => format!("XML-RPC fault: {val:?}"),
                        };
                        return Err(AppError::Odoo(msg));
                    }
                }
                Event::End(_) => return Err(AppError::Odoo("No value in fault".into())),
                Event::Eof => return Err(AppError::Odoo("Unexpected EOF in fault".into())),
                _ => continue,
            }
        }
    }
}

fn parse_method_response(xml: &str) -> AppResult<XmlRpcValue> {
    let mut parser = XmlRpcParser::new(xml);
    parser.parse_response()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Perform an XML-RPC call against an Odoo instance.
///
/// * `client`   – shared reqwest client
/// * `url`      – base URL of the Odoo instance, e.g. `https://mycompany.odoo.com`
/// * `endpoint` – XML-RPC endpoint path, e.g. `/xmlrpc/2/common`
/// * `method`   – XML-RPC method name, e.g. `authenticate`
/// * `args`     – positional parameters wrapped in `XmlRpcValue`
pub async fn call_xmlrpc(
    client: &reqwest::Client,
    url: &str,
    endpoint: &str,
    method: &str,
    args: Vec<XmlRpcValue>,
) -> AppResult<XmlRpcValue> {
    let full_url = format!("{}{}", url.trim_end_matches('/'), endpoint);
    let body = build_method_call(method, &args)?;

    log::debug!("XML-RPC POST {full_url} method={method}");
    log::trace!("XML-RPC request body:\n{body}");

    let resp = client
        .post(&full_url)
        .header("Content-Type", "text/xml")
        .body(body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await?;

    if !status.is_success() {
        return Err(AppError::Odoo(format!(
            "HTTP {status} from {full_url}: {text}"
        )));
    }

    log::debug!("XML-RPC response ({} bytes) for {method}", text.len());
    log::trace!("XML-RPC response body:\n{text}");

    if !text.trim_start().starts_with('<') {
        return Err(AppError::Odoo(format!(
            "Non-XML response from {full_url}: {}",
            &text[..text.len().min(200)]
        )));
    }

    parse_method_response(&text)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_call() {
        let xml = build_method_call(
            "authenticate",
            &[
                XmlRpcValue::String("mydb".into()),
                XmlRpcValue::String("admin".into()),
                XmlRpcValue::String("secret".into()),
                XmlRpcValue::Struct(HashMap::new()),
            ],
        )
        .unwrap();
        assert!(xml.contains("<methodCall>"));
        assert!(xml.contains("<methodName>authenticate</methodName>"));
    }

    #[test]
    fn test_parse_int_response() {
        let xml = r#"<?xml version="1.0"?>
<methodResponse><params><param><value><int>42</int></value></param></params></methodResponse>"#;
        let val = parse_method_response(xml).unwrap();
        assert_eq!(val.as_i64(), Some(42));
    }

    #[test]
    fn test_parse_fault() {
        let xml = r#"<?xml version="1.0"?>
<methodResponse><fault><value><struct>
<member><name>faultCode</name><value><int>1</int></value></member>
<member><name>faultString</name><value><string>Access denied</string></value></member>
</struct></value></fault></methodResponse>"#;
        let err = parse_method_response(xml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Access denied"), "got: {msg}");
    }

    #[test]
    fn test_parse_boolean_false_as_empty_many2one() {
        // Odoo returns False for empty many2one fields
        let val = XmlRpcValue::Bool(false);
        assert_eq!(val.as_many2one(), None);
    }
}
