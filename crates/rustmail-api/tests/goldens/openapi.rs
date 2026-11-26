//! A checker for the subset of OpenAPI 3.1 that `docs/api.yaml` uses.
//!
//! Objects are closed: a response field the spec does not name fails, the
//! same as a missing required one, so a handler cannot grow a field the
//! spec does not document. A schema keyword the checker does not know also
//! fails, so the spec cannot start relying on a keyword that is silently
//! skipped. Annotations (`description`, `example`, ...) are ignored.
//!
//! It checks what a response carries: status, content type and JSON body.
//! Response headers and the request side are outside its scope.
//!
//! Shared by the goldens and by the crate's unit tests, so it depends on
//! nothing but `serde_json` and `yaml-rust2`.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use serde_json::{Map, Value};
use yaml_rust2::{Yaml, YamlLoader};

const SPEC_SOURCE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/api.yaml"));
const ANNOTATIONS: &[&str] = &["description", "example", "examples", "default", "summary"];
const KEYWORDS: &[&str] = &[
  "$ref",
  "type",
  "required",
  "properties",
  "items",
  "allOf",
  "oneOf",
  "enum",
  "const",
  "minimum",
  "maximum",
  "maxItems",
  "format",
];
const HTTP_METHODS: &[&str] = &["get", "put", "post", "delete", "patch", "head", "options"];
const JSON: &str = "application/json";

static SPEC: LazyLock<Spec> = LazyLock::new(|| Spec::parse(SPEC_SOURCE));

/// `docs/api.yaml`, parsed once.
pub fn spec() -> &'static Spec {
  &SPEC
}

/// The parsed spec.
pub struct Spec {
  root: Value,
}

/// A documented operation: an HTTP method on a path template.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Operation {
  pub method: String,
  pub path: String,
}

impl std::fmt::Display for Operation {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{} {}", self.method, self.path)
  }
}

impl Spec {
  fn parse(source: &str) -> Self {
    let documents = YamlLoader::load_from_str(source).expect("docs/api.yaml is valid YAML");
    let root = to_json(documents.first().expect("docs/api.yaml holds a document"));
    Self { root }
  }

  /// Every operation under `paths`.
  pub fn operations(&self) -> BTreeSet<Operation> {
    let mut operations = BTreeSet::new();
    for (path, item) in object(&self.root["paths"]) {
      for method in object(item).keys() {
        if HTTP_METHODS.contains(&method.as_str()) {
          operations.insert(Operation {
            method: method.to_uppercase(),
            path: path.clone(),
          });
        }
      }
    }
    operations
  }

  /// The documented responses of `operation`: each status with each
  /// content type it documents, or `None` for a status without a body.
  pub fn responses(&self, operation: &Operation) -> BTreeSet<(u16, Option<String>)> {
    let mut responses = BTreeSet::new();
    for (status, response) in object(&self.operation(operation)["responses"]) {
      let status: u16 = status
        .parse()
        .unwrap_or_else(|_| panic!("{operation} documents a non-numeric status {status}"));
      match self.resolve(response).get("content") {
        Some(content) => responses.extend(
          object(content)
            .keys()
            .map(|media_type| (status, Some(media_type.clone()))),
        ),
        None => {
          responses.insert((status, None));
        }
      }
    }
    responses
  }

  /// The operation a request for the concrete `path` falls under.
  pub fn operation_for(&self, method: &str, path: &str) -> Option<Operation> {
    self
      .operations()
      .into_iter()
      .find(|operation| operation.method == method && template_matches(&operation.path, path))
  }

  /// Checks a response against what the spec documents for `operation`,
  /// and returns the documented content type it matched.
  ///
  /// `body` is checked against the schema only when it is JSON; pass an
  /// encoded (compressed) body as `None` to check the status and content
  /// type alone.
  pub fn check_response(
    &self,
    operation: &Operation,
    status: u16,
    content_type: Option<&str>,
    body: Option<&[u8]>,
  ) -> Result<Option<String>, String> {
    let documented = self.operation(operation)["responses"]
      .get(status.to_string())
      .ok_or_else(|| format!("{operation} does not document {status}"))?;
    let response = self.resolve(documented);
    let Some(content) = response.get("content") else {
      return match body {
        Some(bytes) if !bytes.is_empty() => Err(format!(
          "{operation} documents no body for {status}, got {} bytes",
          bytes.len()
        )),
        _ => Ok(None),
      };
    };
    let media_type = content_type
      .map(|value| value.split(';').next().unwrap_or(value).trim())
      .ok_or_else(|| format!("{operation} {status} has no content type"))?;
    let range = media_type
      .split_once('/')
      .map(|(kind, _)| format!("{kind}/*"))
      .unwrap_or_default();
    let (documented_type, media) = [media_type, range.as_str()]
      .into_iter()
      .find_map(|key| content.get(key).map(|media| (key.to_string(), media)))
      .ok_or_else(|| {
        format!("{operation} does not document content type {media_type} for {status}")
      })?;
    let Some(bytes) = body.filter(|_| media_type == JSON) else {
      return Ok(Some(documented_type));
    };
    let schema = media
      .get("schema")
      .unwrap_or_else(|| panic!("{operation} {status} documents JSON without a schema"));
    let value: Value = serde_json::from_slice(bytes)
      .map_err(|error| format!("{operation} {status} body is not JSON: {error}"))?;
    self
      .validate(schema, &value, "$")
      .map(|()| Some(documented_type))
      .map_err(|error| format!("{operation} {status}: {error}"))
  }

  /// Validates `value` against the schema `reference` names
  /// (`#/components/schemas/...`).
  pub fn validate_against(&self, reference: &str, value: &Value) -> Result<(), String> {
    self.validate(&serde_json::json!({ "$ref": reference }), value, "$")
  }

  fn operation(&self, operation: &Operation) -> &Value {
    &self.root["paths"][&operation.path][operation.method.to_lowercase()]
  }

  fn resolve<'a>(&'a self, node: &'a Value) -> &'a Value {
    match node.get("$ref").and_then(Value::as_str) {
      Some(reference) => self.resolve(self.pointer(reference)),
      None => node,
    }
  }

  fn pointer(&self, reference: &str) -> &Value {
    let pointer = reference
      .strip_prefix('#')
      .unwrap_or_else(|| panic!("only local references are supported: {reference}"));
    self
      .root
      .pointer(pointer)
      .unwrap_or_else(|| panic!("dangling reference {reference}"))
  }

  fn validate(&self, schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    let schema = self.resolve(schema);
    assert!(
      schema.is_object(),
      "the schema at {at} is not an object: {schema}"
    );
    self.validate_keywords(schema, value, at)?;
    if let Value::Object(fields) = value {
      let mut documented = BTreeSet::new();
      if self.collect_properties(schema, &mut documented)
        && let Some(name) = fields
          .keys()
          .find(|name| !documented.contains(name.as_str()))
      {
        return Err(format!("{at}.{name} is not documented"));
      }
    }
    Ok(())
  }

  fn validate_keywords(&self, schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    for (keyword, argument) in object(schema) {
      if ANNOTATIONS.contains(&keyword.as_str()) {
        continue;
      }
      assert!(
        KEYWORDS.contains(&keyword.as_str()),
        "schema keyword {keyword} at {at} is not supported by the contract checker"
      );
      match keyword.as_str() {
        "type" => check_type(argument, value, at)?,
        "required" => {
          let missing = argument
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find(|name| value.is_object() && value.get(name).is_none());
          if let Some(name) = missing {
            return Err(format!("{at}.{name} is required"));
          }
        }
        "properties" => {
          for (name, property) in object(argument) {
            if let Some(field) = value.as_object().and_then(|fields| fields.get(name)) {
              self.validate(property, field, &format!("{at}.{name}"))?;
            }
          }
        }
        "items" => {
          for (index, item) in value.as_array().into_iter().flatten().enumerate() {
            self.validate(argument, item, &format!("{at}[{index}]"))?;
          }
        }
        "allOf" => {
          for part in argument.as_array().into_iter().flatten() {
            self.validate_keywords(self.resolve(part), value, at)?;
          }
        }
        "oneOf" => {
          let branches = argument.as_array().into_iter().flatten();
          let matching = branches
            .filter(|branch| self.validate(branch, value, at).is_ok())
            .count();
          if matching != 1 {
            return Err(format!(
              "{at} = {value} matches {matching} oneOf branches, not 1"
            ));
          }
        }
        "enum"
          if !argument
            .as_array()
            .is_some_and(|allowed| allowed.contains(value)) =>
        {
          return Err(format!("{at} = {value} is not one of {argument}"));
        }
        "const" if argument != value => {
          return Err(format!("{at} = {value}, expected {argument}"));
        }
        "minimum" if value.is_number() && value.as_f64() < argument.as_f64() => {
          return Err(format!("{at} = {value} is below {argument}"));
        }
        "maximum" if value.is_number() && value.as_f64() > argument.as_f64() => {
          return Err(format!("{at} = {value} is above {argument}"));
        }
        "maxItems" => {
          let limit = argument.as_u64().expect("maxItems is a count");
          if value
            .as_array()
            .is_some_and(|items| items.len() as u64 > limit)
          {
            return Err(format!("{at} has more than {limit} items"));
          }
        }
        "format" => check_format(argument, value, at)?,
        _ => {}
      }
    }
    Ok(())
  }

  /// Gathers the property names an object schema documents, through `allOf`.
  /// Returns false when the schema does not describe an object's fields.
  fn collect_properties<'a>(
    &'a self,
    schema: &'a Value,
    documented: &mut BTreeSet<&'a str>,
  ) -> bool {
    let schema = self.resolve(schema);
    let mut describes_fields = false;
    if let Some(properties) = schema.get("properties") {
      documented.extend(object(properties).keys().map(String::as_str));
      describes_fields = true;
    }
    for part in schema
      .get("allOf")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
    {
      describes_fields |= self.collect_properties(part, documented);
    }
    describes_fields
  }
}

/// Whether `path` is an instance of `template`, where `{name}` matches one
/// non-empty segment.
pub fn template_matches(template: &str, path: &str) -> bool {
  let template: Vec<&str> = template.split('/').collect();
  let path: Vec<&str> = path.split('/').collect();
  template.len() == path.len()
    && template.iter().zip(&path).all(|(expected, actual)| {
      let parameter = expected.starts_with('{') && expected.ends_with('}');
      (parameter && !actual.is_empty()) || expected == actual
    })
}

fn object(value: &Value) -> &Map<String, Value> {
  static EMPTY: LazyLock<Map<String, Value>> = LazyLock::new(Map::new);
  value.as_object().unwrap_or(&EMPTY)
}

fn to_json(yaml: &Yaml) -> Value {
  match yaml {
    Yaml::Null => Value::Null,
    Yaml::Boolean(flag) => Value::Bool(*flag),
    Yaml::Integer(number) => Value::from(*number),
    Yaml::Real(text) => text
      .parse::<f64>()
      .ok()
      .and_then(serde_json::Number::from_f64)
      .map(Value::Number)
      .unwrap_or_else(|| panic!("docs/api.yaml holds an unreadable number {text}")),
    Yaml::String(text) => Value::String(text.clone()),
    Yaml::Array(items) => Value::Array(items.iter().map(to_json).collect()),
    Yaml::Hash(entries) => Value::Object(
      entries
        .iter()
        .map(|(key, value)| (scalar_key(key), to_json(value)))
        .collect(),
    ),
    Yaml::Alias(_) | Yaml::BadValue => panic!("docs/api.yaml uses an unsupported YAML node"),
  }
}

fn scalar_key(key: &Yaml) -> String {
  match key {
    Yaml::String(text) | Yaml::Real(text) => text.clone(),
    Yaml::Integer(number) => number.to_string(),
    Yaml::Boolean(flag) => flag.to_string(),
    other => panic!("docs/api.yaml uses a non-scalar key {other:?}"),
  }
}

fn check_type(argument: &Value, value: &Value, at: &str) -> Result<(), String> {
  let allowed: Vec<&str> = match argument {
    Value::String(name) => vec![name.as_str()],
    Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
    _ => panic!("type at {at} is neither a name nor a list"),
  };
  let fits = |name: &str| match name {
    "null" => value.is_null(),
    "boolean" => value.is_boolean(),
    "integer" => value.is_i64() || value.is_u64(),
    "number" => value.is_number(),
    "string" => value.is_string(),
    "array" => value.is_array(),
    "object" => value.is_object(),
    other => panic!("unknown type {other} at {at}"),
  };
  if allowed.iter().any(|name| fits(name)) {
    Ok(())
  } else {
    Err(format!("{at} = {value} is not of type {argument}"))
  }
}

fn check_format(argument: &Value, value: &Value, at: &str) -> Result<(), String> {
  let Some(text) = value.as_str() else {
    return Ok(());
  };
  match argument.as_str() {
    Some("date-time") if !is_utc_second(text) => {
      Err(format!("{at} = {text} is not YYYY-MM-DDTHH:MM:SSZ"))
    }
    Some("date-time" | "int64") => Ok(()),
    other => panic!("format {other:?} at {at} is not supported by the contract checker"),
  }
}

fn is_utc_second(text: &str) -> bool {
  const SHAPE: &[u8; 20] = b"dddd-dd-ddTdd:dd:ddZ";
  text.len() == SHAPE.len()
    && text
      .bytes()
      .zip(SHAPE)
      .all(|(byte, expected)| match expected {
        b'd' => byte.is_ascii_digit(),
        other => byte == *other,
      })
}
