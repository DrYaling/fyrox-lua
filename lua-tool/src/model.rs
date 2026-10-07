use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub schema: u32,
    pub source: String,
    pub types: Vec<TypeEntry>,
    pub functions: Vec<FunctionEntry>,
    pub methods: Vec<MethodEntry>,
    pub components: Vec<ComponentEntry>,
    pub data_types: Vec<DataTypeEntry>,
}
#[derive(Debug, Serialize)]
pub struct TypeEntry {
    pub name: String,
    pub kind: String,
    pub fields: Vec<String>,
    pub status: &'static str,
}
#[derive(Debug, Serialize)]
pub struct FunctionEntry {
    pub name: String,
    pub signature: String,
    pub status: &'static str,
}
#[derive(Debug, Serialize)]
pub struct MethodEntry {
    pub owner: String,
    pub name: String,
    pub signature: String,
    pub status: &'static str,
}
#[derive(Debug, Serialize)]
pub struct UnsupportedEntry {
    pub source: String,
    pub item: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ComponentEntry {
    pub name: String,
    pub lua_namespace: String,
    pub source_type: String,
    pub category: String,
    pub methods: Vec<String>,
    pub traits: Vec<String>,
    pub status: &'static str,
}
#[derive(Debug, Clone, Serialize)]
pub struct DataTypeEntry {
    pub name: String,
    pub lua_namespace: String,
    pub fields: Vec<String>,
    pub constructors: Vec<String>,
}
