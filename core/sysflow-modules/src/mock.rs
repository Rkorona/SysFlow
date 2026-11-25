//! A fake module backed by a JSON file. It exercises every item type, the read-back
//! `mismatch` path (mock.level = 100 is clamped to 95) and a module-enforced lock
//! (mock.cores[0] must stay on), so the whole stack can be tested without a device.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use sysflow_core::error::{Error, Result};
use sysflow_core::module::{ApplyResult, Caps, Module};
use sysflow_core::schema::{ItemType, Risk, Schema, SchemaItem};

pub struct MockModule {
    state_path: PathBuf,
}

impl MockModule {
    pub fn new(state_path: PathBuf) -> Self {
        MockModule { state_path }
    }

    fn defaults() -> Map<String, Value> {
        json!({
            "mock.flag": false,
            "mock.level": 50,
            "mock.mode": "a",
            "mock.cores": [1, 1, 1, 1],
            "mock.temp": 420
        })
        .as_object()
        .cloned()
        .unwrap_or_default()
    }

    fn load(&self) -> Map<String, Value> {
        let mut state = Self::defaults();
        if let Ok(text) = fs::read_to_string(&self.state_path) {
            if let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&text) {
                for (k, v) in saved {
                    state.insert(k, v);
                }
            }
        }
        state
    }

    fn save(&self, state: &Map<String, Value>) -> Result<()> {
        if let Some(dir) = self.state_path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(state).map_err(|e| Error::io(e.to_string()))?;
        fs::write(&self.state_path, text)?;
        Ok(())
    }
}

impl Module for MockModule {
    fn id(&self) -> &'static str {
        "mock"
    }

    fn probe(&self) -> Caps {
        Caps::supported(&["flag", "level", "mode", "cores"])
    }

    fn schema(&self) -> Schema {
        Schema {
            module: "mock".to_string(),
            supported: true,
            items: vec![
                SchemaItem::new("mock.flag", ItemType::Bool).persist(true),
                SchemaItem::new("mock.level", ItemType::IntRange)
                    .range(0, 100, 5)
                    .unit("%")
                    .persist(true),
                SchemaItem::new("mock.mode", ItemType::Enum)
                    .options(vec![json!("a"), json!("b"), json!("c")])
                    .persist(true),
                SchemaItem::new("mock.cores", ItemType::BoolList)
                    .count(4)
                    .locked(vec![0])
                    .risk(Risk::Medium)
                    .persist(true),
                SchemaItem::new("mock.temp", ItemType::Readonly).unit("0.1C"),
            ],
        }
    }

    fn collect(&self, keys: Option<&[String]>) -> Result<Value> {
        let state = self.load();
        let out: Map<String, Value> = match keys {
            None => state,
            Some(keys) => {
                let mut m = Map::new();
                for k in keys {
                    if let Some(v) = state.get(k) {
                        m.insert(k.clone(), v.clone());
                    }
                }
                m
            }
        };
        Ok(Value::Object(out))
    }

    fn apply(&self, req: &Map<String, Value>) -> Result<ApplyResult> {
        let mut state = self.load();
        for (k, v) in req {
            let mut value = v.clone();
            if k == "mock.level" && value.as_i64() == Some(100) {
                // simulate the kernel / thermal HAL refusing the top step
                value = json!(95);
            }
            if k == "mock.cores" {
                let first_on = value
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|x| x.as_i64())
                    == Some(1);
                if !first_on {
                    return Err(Error::invalid_arg("mock.cores[0] is locked on"));
                }
            }
            state.insert(k.clone(), value);
        }
        self.save(&state)?;
        let mut applied = Map::new();
        for k in req.keys() {
            if let Some(v) = state.get(k) {
                applied.insert(k.clone(), v.clone());
            }
        }
        Ok(ApplyResult::from_readback(req, applied))
    }
}
