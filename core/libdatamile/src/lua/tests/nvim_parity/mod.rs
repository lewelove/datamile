use mlua::LuaSerdeExt;
use serde::Deserialize;

use crate::lua::LuaEngine;

const SHIM: &str = include_str!("shim.lua");

#[derive(Deserialize, Debug)]
struct TestResult {
    name: String,
    ok: bool,
    err: Option<String>,
}

fn run_spec(name: &str, code: &str) {
    if code.trim().is_empty() {
        return;
    }
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    engine
        .lua
        .load(SHIM)
        .set_name("shim.lua")
        .exec()
        .expect("Failed to initialize Neovim parity test shim");

    let exec_res = engine.lua.load(code).set_name(name).exec();
    assert!(
        exec_res.is_ok(),
        "Spec syntax error in '{name}': {:?}",
        exec_res.err()
    );

    let results_val: mlua::Value = engine
        .lua
        .globals()
        .get("__DATAMILE_TEST_RESULTS")
        .expect("Missing __DATAMILE_TEST_RESULTS in global table");

    let results: Vec<TestResult> = engine
        .lua
        .from_value(results_val)
        .expect("Failed to deserialize test results");

    let failed: Vec<&TestResult> = results.iter().filter(|r| !r.ok).collect();
    if !failed.is_empty() {
        let mut err_msg = format!("{} test(s) failed in '{name}':\n", failed.len());
        for f in &failed {
            err_msg.push_str(&format!(
                "  [FAIL] {} -> {}\n",
                f.name,
                f.err.as_deref().unwrap_or("unknown error")
            ));
        }
        panic!("{err_msg}");
    }
}

#[test]
fn test_nvim_fs_spec() {
    let code = include_str!("specs/modded/fs_spec.lua");
    run_spec("fs_spec.lua", code);
}

#[test]
fn test_nvim_list_spec() {
    let code = include_str!("specs/original/list_spec.lua");
    run_spec("list_spec.lua", code);
}

#[test]
fn test_nvim_json_spec() {
    let code = include_str!("specs/modded/json_spec.lua");
    run_spec("json_spec.lua", code);
}

#[test]
fn test_nvim_vim_spec() {
    let code = include_str!("specs/modded/vim_spec.lua");
    run_spec("vim_spec.lua", code);
}
