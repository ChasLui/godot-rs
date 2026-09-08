//! Data model for `extension_api.json`.
//!
//! Only the parts the generator consumes are modelled; unknown fields are ignored so a newer
//! dump does not break parsing.

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Api {
    pub header: Header,
    pub global_enums: Vec<GlobalEnum>,
    pub utility_functions: Vec<UtilityFunction>,
    pub classes: Vec<Class>,
    pub singletons: Vec<Singleton>,
}

#[derive(Deserialize)]
pub struct Header {
    pub version_full_name: String,
    pub version_major: u32,
    pub version_minor: u32,
    pub precision: String,
}

#[derive(Deserialize)]
pub struct GlobalEnum {
    pub name: String,
    #[serde(default)]
    pub is_bitfield: bool,
    pub values: Vec<EnumValue>,
}

#[derive(Deserialize)]
pub struct EnumValue {
    pub name: String,
    pub value: i64,
}

#[derive(Deserialize)]
pub struct UtilityFunction {
    pub name: String,
    #[serde(default)]
    pub return_type: Option<String>,
    pub hash: i64,
    #[serde(default)]
    pub is_vararg: bool,
    #[serde(default)]
    pub arguments: Vec<Argument>,
}

#[derive(Deserialize)]
pub struct Class {
    pub name: String,
    #[serde(default)]
    pub is_refcounted: bool,
    #[serde(default)]
    pub is_instantiable: bool,
    #[serde(default)]
    pub inherits: Option<String>,
    pub api_type: String,
    #[serde(default)]
    pub methods: Vec<Method>,
    #[serde(default)]
    pub properties: Vec<Property>,
    #[serde(default)]
    pub signals: Vec<Signal>,
}

#[derive(Deserialize)]
pub struct Method {
    pub name: String,
    #[serde(default)]
    pub is_const: bool,
    #[serde(default)]
    pub is_vararg: bool,
    #[serde(default)]
    pub is_static: bool,
    #[serde(default)]
    pub is_virtual: bool,
    /// Absent for virtual methods, which have no bindable implementation.
    #[serde(default)]
    pub hash: Option<i64>,
    #[serde(default)]
    pub return_value: Option<ReturnValue>,
    #[serde(default)]
    pub arguments: Vec<Argument>,
}

#[derive(Deserialize)]
pub struct ReturnValue {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub meta: Option<String>,
}

#[derive(Deserialize)]
pub struct Argument {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub meta: Option<String>,
    #[serde(default)]
    pub default_value: Option<String>,
}

#[derive(Deserialize)]
pub struct Property {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct Signal {
    pub name: String,
    #[serde(default)]
    pub arguments: Vec<Argument>,
}

#[derive(Deserialize)]
pub struct Singleton {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
}

impl Api {
    pub fn load(path: &str) -> Self {
        let src =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        serde_json::from_str(&src).unwrap_or_else(|e| panic!("cannot parse {path}: {e}"))
    }
}
