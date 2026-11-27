use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{Error, Result};

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Bool,
    BoolList,
    IntRange,
    IntEnum,
    Enum,
    Text,
    Readonly,
    Action,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    Cluster,
    Cpu,
    App,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Clone, Debug)]
pub struct SchemaItem {
    pub id: String,
    #[serde(rename = "type")]
    pub ty: ItemType,
    pub scope: Scope,
    pub risk: Risk,
    pub persist: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locked: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_key: Option<String>,
}

impl SchemaItem {
    pub fn new(id: &str, ty: ItemType) -> Self {
        SchemaItem {
            id: id.to_string(),
            ty,
            scope: Scope::Global,
            risk: Risk::Low,
            persist: false,
            count: None,
            locked: Vec::new(),
            options: None,
            min: None,
            max: None,
            step: None,
            max_len: None,
            unit: None,
            depends_on: Vec::new(),
            title_key: None,
        }
    }

    pub fn scope(mut self, s: Scope) -> Self {
        self.scope = s;
        self
    }
    pub fn with_type(mut self, ty: ItemType) -> Self {
        self.ty = ty;
        self
    }
    pub fn risk(mut self, r: Risk) -> Self {
        self.risk = r;
        self
    }
    pub fn persist(mut self, p: bool) -> Self {
        self.persist = p;
        self
    }
    pub fn count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }
    pub fn locked(mut self, l: Vec<usize>) -> Self {
        self.locked = l;
        self
    }
    pub fn options(mut self, o: Vec<Value>) -> Self {
        self.options = Some(o);
        self
    }
    pub fn range(mut self, min: i64, max: i64, step: i64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self.step = Some(step);
        self
    }
    pub fn max_len(mut self, n: usize) -> Self {
        self.max_len = Some(n);
        self
    }
    pub fn unit(mut self, u: &str) -> Self {
        self.unit = Some(u.to_string());
        self
    }
    pub fn depends_on(mut self, d: Vec<String>) -> Self {
        self.depends_on = d;
        self
    }
    pub fn title_key(mut self, k: &str) -> Self {
        self.title_key = Some(k.to_string());
        self
    }

    /// Validate a value for `set`. For non-global scopes the value must be an
    /// object keyed by cluster/cpu/app, and every entry is validated as a scalar.
    pub fn validate_value(&self, v: &Value) -> Result<()> {
        if self.ty == ItemType::Readonly {
            return Err(Error::invalid_arg(format!("{} is read-only", self.id)));
        }
        match self.scope {
            Scope::Global => self.validate_scalar(v),
            _ => {
                let obj = v.as_object().ok_or_else(|| {
                    Error::invalid_arg(format!(
                        "{}: expected an object keyed by {:?}",
                        self.id, self.scope
                    ))
                })?;
                for (sub, val) in obj {
                    self.validate_scalar(val).map_err(|e| {
                        Error::new(e.code, format!("{}[{}]: {}", self.id, sub, e.msg))
                    })?;
                }
                Ok(())
            }
        }
    }

    fn validate_scalar(&self, v: &Value) -> Result<()> {
        let bad = |what: &str| Error::invalid_arg(format!("{}: {}", self.id, what));
        match self.ty {
            ItemType::Bool => {
                if v.is_boolean() {
                    Ok(())
                } else {
                    Err(bad("expected true/false"))
                }
            }
            ItemType::Action => {
                if v.as_bool() == Some(true) {
                    Ok(())
                } else {
                    Err(bad("actions are triggered with true"))
                }
            }
            ItemType::BoolList => {
                let arr = v
                    .as_array()
                    .ok_or_else(|| bad("expected an array of 0/1"))?;
                if let Some(n) = self.count {
                    if arr.len() != n {
                        return Err(bad(&format!("expected {n} entries, got {}", arr.len())));
                    }
                }
                for e in arr {
                    match e.as_i64() {
                        Some(0) | Some(1) => {}
                        _ => return Err(bad("entries must be 0 or 1")),
                    }
                }
                Ok(())
            }
            ItemType::IntRange => {
                let i = v.as_i64().ok_or_else(|| bad("expected an integer"))?;
                if let Some(min) = self.min {
                    if i < min {
                        return Err(bad(&format!("{i} is below minimum {min}")));
                    }
                }
                if let Some(max) = self.max {
                    if i > max {
                        return Err(bad(&format!("{i} is above maximum {max}")));
                    }
                }
                if let (Some(step), Some(min)) = (self.step, self.min) {
                    if step > 0 && (i - min) % step != 0 {
                        return Err(bad(&format!("{i} is not on a step of {step}")));
                    }
                }
                Ok(())
            }
            ItemType::IntEnum => {
                let i = v.as_i64().ok_or_else(|| bad("expected an integer"))?;
                let ok = self
                    .options
                    .as_ref()
                    .map(|o| o.iter().any(|x| x.as_i64() == Some(i)))
                    .unwrap_or(false);
                if ok {
                    Ok(())
                } else {
                    Err(bad(&format!("{i} is not an allowed option")))
                }
            }
            ItemType::Enum => {
                let s = v.as_str().ok_or_else(|| bad("expected a string"))?;
                let ok = self
                    .options
                    .as_ref()
                    .map(|o| o.iter().any(|x| x.as_str() == Some(s)))
                    .unwrap_or(false);
                if ok {
                    Ok(())
                } else {
                    Err(bad(&format!("{s:?} is not an allowed option")))
                }
            }
            ItemType::Text => {
                let s = v.as_str().ok_or_else(|| bad("expected a string"))?;
                if s.is_empty() {
                    return Err(bad("must not be empty"));
                }
                if s.chars().any(char::is_control) {
                    return Err(bad("must not contain control characters"));
                }
                if let Some(n) = self.max_len {
                    if s.len() > n {
                        return Err(bad(&format!("is longer than {n} bytes")));
                    }
                }
                Ok(())
            }
            ItemType::Readonly => Err(bad("read-only")),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Schema {
    pub module: String,
    pub supported: bool,
    pub items: Vec<SchemaItem>,
}

impl Schema {
    pub fn unsupported(module: &str) -> Self {
        Schema {
            module: module.to_string(),
            supported: false,
            items: Vec::new(),
        }
    }

    /// Every key must exist in the schema, must not be read-only, and must have a valid value.
    pub fn validate_request(&self, req: &Map<String, Value>) -> Result<()> {
        for (k, v) in req {
            let item = self
                .items
                .iter()
                .find(|i| &i.id == k)
                .ok_or_else(|| Error::invalid_arg(format!("unknown key: {k}")))?;
            item.validate_value(v)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Schema {
        Schema {
            module: "t".into(),
            supported: true,
            items: vec![
                SchemaItem::new("t.level", ItemType::IntRange).range(0, 100, 5),
                SchemaItem::new("t.mode", ItemType::Enum).options(vec![json!("a"), json!("b")]),
                SchemaItem::new("t.cores", ItemType::BoolList).count(4),
                SchemaItem::new("t.freq", ItemType::IntEnum)
                    .scope(Scope::Cluster)
                    .options(vec![json!(100), json!(200)]),
                SchemaItem::new("t.temp", ItemType::Readonly),
                SchemaItem::new("t.trim", ItemType::Action),
            ],
        }
    }

    fn req(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn accepts_valid_requests() {
        let r = req(json!({
            "t.level": 50, "t.mode": "a", "t.cores": [1, 0, 1, 1],
            "t.freq": {"0": 100, "4": 200}, "t.trim": true
        }));
        assert!(schema().validate_request(&r).is_ok());
    }

    #[test]
    fn rejects_invalid_requests() {
        let bad = [
            json!({"t.level": 51}),
            json!({"t.level": 101}),
            json!({"t.level": -5}),
            json!({"t.level": "5"}),
            json!({"t.mode": "z"}),
            json!({"t.cores": [1, 1]}),
            json!({"t.cores": [1, 2, 1, 1]}),
            json!({"t.freq": 100}),
            json!({"t.freq": {"0": 300}}),
            json!({"t.temp": 1}),
            json!({"t.trim": false}),
            json!({"nope": 1}),
        ];
        for b in bad {
            assert!(
                schema().validate_request(&req(b.clone())).is_err(),
                "should reject {b}"
            );
        }
    }

    #[test]
    fn text_values_are_bounded_and_serialized() {
        let item = SchemaItem::new("t.text", ItemType::Text)
            .scope(Scope::Cluster)
            .max_len(5);
        assert!(item.validate_value(&json!({"0": "1 2:3"})).is_ok());
        assert!(item.validate_value(&json!({"0": ""})).is_err());
        assert!(item.validate_value(&json!({"0": "abcdef"})).is_err());
        assert!(item.validate_value(&json!({"0": "a\u{7}b"})).is_err());
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["type"], "text");
        assert_eq!(v["max_len"], 5);
    }

    #[test]
    fn item_serializes_with_protocol_field_names() {
        let item = SchemaItem::new("cpu.online", ItemType::BoolList)
            .count(8)
            .locked(vec![0])
            .risk(Risk::Medium)
            .persist(true);
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["type"], "bool_list");
        assert_eq!(v["risk"], "medium");
        assert_eq!(v["scope"], "global");
        assert_eq!(v["count"], 8);
        assert!(v.get("options").is_none());
    }
}
