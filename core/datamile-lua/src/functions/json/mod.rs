#[cfg(test)]
mod tests;

use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table, Value};

fn is_empty_dict(tbl: &Table) -> bool {
    tbl.metatable()
        .and_then(|mt| mt.get::<bool>("__is_empty_dict").ok())
        .unwrap_or(false)
}

fn is_nil_sentinel(tbl: &Table) -> bool {
    if let Some(mt) = tbl.metatable()
        && let Ok(tag) = mt.get::<bool>("__is_nil")
    {
        return tag;
    }
    if let Ok(tostring) = tbl.get::<mlua::Function>("__tostring")
        && let Ok(mlua::Value::String(s)) = tostring.call(())
        && s.to_str().is_ok_and(|val| val == "null")
    {
        return true;
    }
    false
}

fn lua_value_to_json(val: Value) -> mlua::Result<serde_json::Value> {
    match val {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        Value::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        Value::Number(n) => serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| mlua::Error::runtime("cannot serialize non-finite number")),
        Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        Value::Table(tbl) => {
            if is_nil_sentinel(&tbl) {
                return Ok(serde_json::Value::Null);
            }
            if is_empty_dict(&tbl) {
                return Ok(serde_json::Value::Object(serde_json::Map::new()));
            }

            let raw_len = tbl.raw_len();
            let mut pair_count = 0usize;
            let mut has_non_sequential_key = false;

            for pair in tbl.pairs::<Value, Value>() {
                let (k, _) = pair?;
                pair_count += 1;
                match k {
                    Value::Integer(i) => {
                        if !usize::try_from(i)
                            .is_ok_and(|idx| (1..=raw_len).contains(&idx))
                        {
                            has_non_sequential_key = true;
                        }
                    }
                    _ => {
                        has_non_sequential_key = true;
                    }
                }
            }

            if pair_count == 0 {
                return Ok(serde_json::Value::Array(Vec::new()));
            }

            if !has_non_sequential_key && pair_count == raw_len {
                let mut arr = Vec::with_capacity(raw_len);
                for i in 1..=raw_len {
                    let elem: Value = tbl.raw_get(i)?;
                    arr.push(lua_value_to_json(elem)?);
                }
                Ok(serde_json::Value::Array(arr))
            } else {
                let mut map = serde_json::Map::with_capacity(pair_count);
                for pair in tbl.pairs::<Value, Value>() {
                    let (k, v) = pair?;
                    let key_str = match k {
                        Value::String(s) => s.to_str()?.to_string(),
                        Value::Integer(i) => i.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Boolean(b) => b.to_string(),
                        _ => {
                            return Err(mlua::Error::runtime(
                                "invalid table key for json serialization",
                            ));
                        }
                    };
                    let json_val = lua_value_to_json(v)?;
                    map.insert(key_str, json_val);
                }
                Ok(serde_json::Value::Object(map))
            }
        }
        _ => Err(mlua::Error::runtime("cannot serialize object")),
    }
}

pub fn register(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    let json_table = lua.create_table()?;
    json_table.set(
        "decode",
        lua.create_function(move |lua, s: String| {
            let val: serde_json::Value =
                serde_json::from_str(&s).map_err(mlua::Error::external)?;
            lua.to_value_with(&val, opts)
        })?,
    )?;

    json_table.set(
        "encode",
        lua.create_function(|_, val: Value| {
            let json_val = lua_value_to_json(val)?;
            serde_json::to_string(&json_val).map_err(mlua::Error::external)
        })?,
    )?;

    datamile_tbl.set("json", json_table)
}
