use hlua::{AnyHashableLuaValue, AnyLuaValue};
use mysql;

use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use twox_hash::XxHash;
use structs::LuaMap;


impl From<mysql::Params> for LuaMap {
    fn from(params: mysql::Params) -> LuaMap {
        match params {
            mysql::Params::Empty => LuaMap::new(),
            mysql::Params::Named(map) => {
                map.into_iter()
                    .map(|(k, v)| (AnyHashableLuaValue::LuaString(k), mysql_value_to_lua(v)))
                    .collect::<HashMap<AnyHashableLuaValue, AnyLuaValue>>()
                    .into()
            },
            mysql::Params::Positional(_) => unimplemented!(),
        }
    }
}

impl Into<mysql::Params> for LuaMap {
    fn into(self) -> mysql::Params {
        if self.is_empty() {
            mysql::Params::Empty
        } else {
            let mut params: HashMap<String, mysql::Value, BuildHasherDefault<XxHash>> = HashMap::default();

            for (k, v) in self {
                if let AnyHashableLuaValue::LuaString(k) = k {
                    params.insert(k, lua_to_mysql_value(v));
                } else {
                    panic!("unsupported keys in map");
                }
            }

            mysql::Params::Named(params)
        }
    }
}

fn lua_to_mysql_value(value: AnyLuaValue) -> mysql::Value {
    match value {
        AnyLuaValue::LuaString(x) => mysql::Value::Bytes(x.into_bytes()),
        AnyLuaValue::LuaAnyString(x) => mysql::Value::Bytes(x.0),
        AnyLuaValue::LuaNumber(v) => if v % 1f64 == 0f64 {
            mysql::Value::Int(v as i64)
        } else {
            mysql::Value::Float(v)
        },
        AnyLuaValue::LuaBoolean(x) => mysql::Value::Int(if x { 1 } else { 0 }),
        AnyLuaValue::LuaArray(_x) => unimplemented!(),
        AnyLuaValue::LuaNil => mysql::Value::NULL,
        AnyLuaValue::LuaOther => unimplemented!(),
    }
}

pub fn mysql_value_to_lua(value: mysql::Value) -> AnyLuaValue {
    use mysql::Value::*;
    match value {
        NULL => AnyLuaValue::LuaNil,
        Bytes(bytes) => AnyLuaValue::LuaString(String::from_utf8(bytes).unwrap()),
        Int(i) => AnyLuaValue::LuaNumber(i as f64),
        UInt(i) => AnyLuaValue::LuaNumber(i as f64),
        Float(i) => AnyLuaValue::LuaNumber(i),
        Date(_, _, _, _, _, _, _) => unimplemented!(),
        Time(_, _, _, _, _, _) => unimplemented!(),
    }
}
