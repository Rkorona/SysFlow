//! Generate the TypeScript and Kotlin protocol models from one Rust description.
//!
//! The module binary, the APK and the WebUI all speak the protocol in `docs/protocol.md`. Keeping
//! the wire types in one small Rust model (and generated files marked "do not edit") is what stops
//! the three ends from drifting.
//!
//! Usage:
//!   cargo run -p sysflow-types -- --kotlin <file>            # APK (current)
//!   cargo run -p sysflow-types -- --ts <file> --kotlin <file> # both, e.g. a future typed client
//!
//! Either output may be requested alone; asking for neither is a usage error.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy)]
enum Ty {
    Str,
    Int,
    Long,
    Bool,
    Json,
    JsonObject,
    Ref(&'static str),
    List(&'static Ty),
    Map(&'static Ty),
}

static STR: Ty = Ty::Str;
static INT: Ty = Ty::Int;
static LONG: Ty = Ty::Long;
static BOOL: Ty = Ty::Bool;
static JSON: Ty = Ty::Json;
static JSON_OBJECT: Ty = Ty::JsonObject;
static STR_LIST: Ty = Ty::List(&STR);
static INT_LIST: Ty = Ty::List(&INT);
static STR_MAP_LIST: Ty = Ty::Map(&Ty::List(&STR));
static ERROR_MAP: Ty = Ty::Map(&Ty::Ref("ProtocolError"));

struct Field {
    name: &'static str,
    ty: Ty,
    optional: bool,
    doc: &'static str,
}

fn field(name: &'static str, ty: Ty) -> Field {
    Field {
        name,
        ty,
        optional: false,
        doc: "",
    }
}

fn optional(name: &'static str, ty: Ty, doc: &'static str) -> Field {
    Field {
        name,
        ty,
        optional: true,
        doc,
    }
}

enum Def {
    Struct {
        name: &'static str,
        doc: &'static str,
        fields: Vec<Field>,
    },
    Enum {
        name: &'static str,
        variants: Vec<(&'static str, &'static str)>,
    },
}

fn defs() -> Vec<Def> {
    vec![
        Def::Enum {
            name: "ErrorCode",
            variants: vec![
                ("Unsupported", "unsupported"),
                ("Permission", "permission"),
                ("InvalidArg", "invalid_arg"),
                ("Busy", "busy"),
                ("Io", "io"),
                ("SafeMode", "safe_mode"),
            ],
        },
        Def::Enum {
            name: "ItemType",
            variants: vec![
                ("Bool", "bool"),
                ("BoolList", "bool_list"),
                ("IntRange", "int_range"),
                ("IntEnum", "int_enum"),
                ("Enum", "enum"),
                ("Text", "text"),
                ("Readonly", "readonly"),
                ("Action", "action"),
            ],
        },
        Def::Enum {
            name: "Scope",
            variants: vec![
                ("Global", "global"),
                ("Cluster", "cluster"),
                ("Cpu", "cpu"),
                ("App", "app"),
            ],
        },
        Def::Enum {
            name: "Risk",
            variants: vec![
                ("Low", "low"),
                ("Medium", "medium"),
                ("High", "high"),
            ],
        },
        Def::Struct {
            name: "ProtocolError",
            doc: "The `error` object of a failed envelope.",
            fields: vec![
                field("code", Ty::Ref("ErrorCode")),
                field("msg", STR),
                optional("detail", JSON, "Extra context for the UI / diagnostics."),
            ],
        },
        Def::Struct {
            name: "Caps",
            doc: "What `probe()` found for one module.",
            fields: vec![
                field("supported", BOOL),
                field("features", STR_LIST),
                optional("reason", Ty::Str, "Why the module is unsupported."),
            ],
        },
        Def::Struct {
            name: "CapsData",
            doc: "Payload of `sysflow caps`.",
            fields: vec![
                field("sysflow_version", STR),
                field("protocol", INT),
                field("safe_mode", BOOL),
                field("modules", Ty::Ref("ModulesMap")),
            ],
        },
        Def::Struct {
            name: "SchemaItem",
            doc: "One settable / readable item in a module schema.",
            fields: vec![
                field("id", STR),
                field("type", Ty::Ref("ItemType")),
                field("scope", Ty::Ref("Scope")),
                field("risk", Ty::Ref("Risk")),
                field("persist", BOOL),
                optional("count", INT, "Number of entries for `bool_list`."),
                Field { name: "locked", ty: INT_LIST, optional: true, doc: "Indices the UI must not change." },
                optional("options", Ty::List(&JSON), "Allowed values for int_enum / enum."),
                optional("min", LONG, "Minimum for int_range."),
                optional("max", LONG, "Maximum for int_range."),
                optional("step", LONG, "Step for int_range."),
                optional("max_len", INT, "Maximum length for text."),
                optional("unit", STR, "Unit shown by the UI (kHz, 0.1C, ...)."),
                Field { name: "depends_on", ty: STR_LIST, optional: true, doc: "Items that should be set first." },
                optional("title_key", STR, "i18n key; the UI falls back to the id."),
            ],
        },
        Def::Struct {
            name: "Schema",
            doc: "Payload of `sysflow <module> schema`.",
            fields: vec![
                field("module", STR),
                field("supported", BOOL),
                field("items", Ty::List(&Ty::Ref("SchemaItem"))),
                optional("reason", STR, "Why the module is unsupported."),
            ],
        },
        Def::Struct {
            name: "ApplyResult",
            doc: "Payload of `sysflow <module> set`; always the read-back state.",
            fields: vec![
                field("applied", JSON_OBJECT),
                field("requested", JSON_OBJECT),
                field("mismatch", STR_LIST),
            ],
        },
        Def::Struct {
            name: "SnapshotData",
            doc: "Payload of `sysflow snapshot`; module payloads stay dynamic because every module has its own items.",
            fields: vec![
                field("ts", LONG),
                field("safe_mode", BOOL),
                field("modules", JSON_OBJECT),
                optional("settings", JSON_OBJECT, "Persisted per-module settings; used to distinguish static configuration from live effective values."),
                optional("defaults", JSON_OBJECT, "First-run per-module defaults captured by the module manager."),
                field("errors", ERROR_MAP),
            ],
        },
        Def::Struct {
            name: "CheckData",
            doc: "Payload of the read-only `sysflow check`.",
            fields: vec![
                field("clean", BOOL),
                field("drift", STR_MAP_LIST),
                field("errors", ERROR_MAP),
            ],
        },
        Def::Struct {
            name: "VersionData",
            doc: "Payload of `sysflow version`.",
            fields: vec![
                field("sysflow_version", STR),
                field("protocol", INT),
                field("config_version", INT),
            ],
        },
        Def::Struct {
            name: "SafeStatus",
            doc: "Payload of `sysflow safe-mode status|clear`.",
            fields: vec![
                field("safe_mode", BOOL),
                field("boot_count", INT),
                field("threshold", INT),
                field("rescue_marker", BOOL),
            ],
        },
        Def::Struct {
            name: "HistoryData",
            doc: "Payload of `sysflow history`; series stay dynamic (null points are allowed).",
            fields: vec![
                field("module", STR),
                field("step_s", LONG),
                field("daemon_running", BOOL),
                field("series", JSON_OBJECT),
            ],
        },
        Def::Struct {
            name: "DaemonStatus",
            doc: "Payload of `sysflow daemon status`.",
            fields: vec![
                field("running", BOOL),
                field("implemented", BOOL),
            ],
        },
    ]
}

fn main() {
    let mut ts_path = None;
    let mut kotlin_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--ts" => ts_path = args.next().map(PathBuf::from),
            "--kotlin" => kotlin_path = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!("usage: gen-types [--ts <file.ts>] [--kotlin <file.kt>]");
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    if ts_path.is_none() && kotlin_path.is_none() {
        eprintln!("usage: gen-types [--ts <file.ts>] [--kotlin <file.kt>]");
        std::process::exit(2);
    }
    if let Some(path) = &ts_path {
        if let Err(e) = write_file(path, &typescript()) {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        }
    }
    if let Some(path) = &kotlin_path {
        if let Err(e) = write_file(path, &kotlin()) {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

fn write_file(path: &PathBuf, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

fn typescript() -> String {
    let mut out = String::new();
    out.push_str(
        "// Generated by `tools/gen-types` from the Rust protocol model. DO NOT EDIT.\n\
         // Source: core/sysflow-types/src/main.rs\n\n\
         export const PROTOCOL_VERSION = 1;\n\n",
    );
    for def in defs() {
        match def {
            Def::Enum { name, variants } => {
                let values = variants
                    .iter()
                    .map(|(_, wire)| format!("'{wire}'"))
                    .collect::<Vec<_>>()
                    .join(" | ");
                let _ = writeln!(out, "export type {name} = {values};\n");
            }
            Def::Struct { name, doc, fields } => {
                let _ = writeln!(out, "/** {doc} */");
                let _ = writeln!(out, "export interface {name} {{");
                for f in fields {
                    if !f.doc.is_empty() {
                        let _ = writeln!(out, "  /** {} */", f.doc);
                    }
                    let opt = if f.optional { "?" } else { "" };
                    let _ = writeln!(out, "  {}{opt}: {};", f.name, ts_type(&f.ty));
                }
                out.push_str("}\n\n");
            }
        }
    }
    // A couple of hand-written helpers that are awkward to express as a plain struct.
    out.push_str(
        "/** Module id -> capability. */\n\
         export type ModulesMap = Record<string, Caps>;\n\n\
         /** Envelope returned by every command. */\n\
         export interface Envelope<T> {\n\
         \tv: number;\n\
         \tok: boolean;\n\
         \tdata?: T;\n\
         \terror?: ProtocolError;\n\
         }\n\n\
         export type CapsEnvelope = Envelope<CapsData>;\n\
         export type SnapshotEnvelope = Envelope<SnapshotData>;\n\
         export type SchemaEnvelope = Envelope<Schema>;\n\
         export type GetEnvelope = Envelope<Record<string, unknown>>;\n\
         export type SetEnvelope = Envelope<ApplyResult>;\n\
         export type CheckEnvelope = Envelope<CheckData>;\n\
         export type VersionEnvelope = Envelope<VersionData>;\n\
         export type SafeStatusEnvelope = Envelope<SafeStatus>;\n\
         export type HistoryEnvelope = Envelope<HistoryData>;\n\
         export type DaemonEnvelope = Envelope<DaemonStatus>;\n",
    );
    out
}

fn ts_type(ty: &Ty) -> String {
    match ty {
        Ty::Str => "string".into(),
        Ty::Int | Ty::Long => "number".into(),
        Ty::Bool => "boolean".into(),
        Ty::Json => "unknown".into(),
        Ty::JsonObject => "Record<string, unknown>".into(),
        Ty::Ref(name) => (*name).into(),
        Ty::List(inner) => format!("{}[]", ts_type(inner)),
        Ty::Map(inner) => format!("Record<string, {}>", ts_type(inner)),
    }
}

fn kotlin() -> String {
    let mut out = String::new();
    out.push_str(
        "// Generated by `tools/gen-types` from the Rust protocol model. DO NOT EDIT.\n\
         // Source: core/sysflow-types/src/main.rs\n\
         @file:Suppress(\"unused\")\n\n\
         package io.sysflow.core.model\n\n\
         import kotlinx.serialization.SerialName\n\
         import kotlinx.serialization.Serializable\n\
         import kotlinx.serialization.json.JsonElement\n\
         import kotlinx.serialization.json.JsonObject\n\n\
         const val PROTOCOL_VERSION: Int = 1\n\n",
    );
    for def in defs() {
        match def {
            Def::Enum { name, variants } => {
                let _ = writeln!(out, "@Serializable");
                let _ = writeln!(out, "enum class {name} {{");
                for (variant, wire) in variants {
                    let _ = writeln!(out, "    @SerialName(\"{wire}\") {variant},");
                }
                out.push_str("}\n\n");
            }
            Def::Struct { name, doc, fields } => {
                let _ = writeln!(out, "/** {doc} */");
                let _ = writeln!(out, "@Serializable");
                let _ = writeln!(out, "data class {name}(");
                for (i, f) in fields.iter().enumerate() {
                    let comma = if i + 1 == fields.len() { "" } else { "," };
                    let optional_list = f.optional && matches!(f.ty, Ty::List(_));
                    let ty = if f.optional && !optional_list {
                        format!("{}?", kotlin_type(&f.ty))
                    } else {
                        kotlin_type(&f.ty)
                    };
                    let default = if f.optional {
                        format!(" = {}", kotlin_default(&f.ty))
                    } else {
                        String::new()
                    };
                    let _ = writeln!(out, "    val {}: {ty}{default}{comma}", f.name);
                }
                out.push_str(")\n\n");
            }
        }
    }
    out.push_str(
        "/** Module id -> capability. */\n\
         typealias ModulesMap = Map<String, Caps>\n\n\
         /** Envelope returned by every command. */\n\
         @Serializable\n\
         data class Envelope<T>(\n\
         \tval v: Int = PROTOCOL_VERSION,\n\
         \tval ok: Boolean,\n\
         \tval data: T? = null,\n\
         \tval error: ProtocolError? = null,\n\
         )\n\n\
         typealias CapsEnvelope = Envelope<CapsData>\n\
         typealias SnapshotEnvelope = Envelope<SnapshotData>\n\
         typealias SchemaEnvelope = Envelope<Schema>\n\
         typealias GetEnvelope = Envelope<JsonObject>\n\
         typealias SetEnvelope = Envelope<ApplyResult>\n\
         typealias CheckEnvelope = Envelope<CheckData>\n\
         typealias VersionEnvelope = Envelope<VersionData>\n\
         typealias SafeStatusEnvelope = Envelope<SafeStatus>\n\
         typealias HistoryEnvelope = Envelope<HistoryData>\n\
         typealias DaemonEnvelope = Envelope<DaemonStatus>\n",
    );
    out
}

fn kotlin_type(ty: &Ty) -> String {
    match ty {
        Ty::Str => "String".into(),
        Ty::Int => "Int".into(),
        Ty::Long => "Long".into(),
        Ty::Bool => "Boolean".into(),
        Ty::Json => "JsonElement".into(),
        Ty::JsonObject => "JsonObject".into(),
        Ty::Ref(name) => (*name).into(),
        Ty::List(inner) => format!("List<{}>", kotlin_type(inner)),
        Ty::Map(inner) => format!("Map<String, {}>", kotlin_type(inner)),
    }
}

fn kotlin_default(ty: &Ty) -> String {
    match ty {
        Ty::List(_) => "emptyList()".into(),
        _ => "null".into(),
    }
}
