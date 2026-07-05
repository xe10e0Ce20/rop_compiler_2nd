use crate::compiler::Compiler;
use crate::parser;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

/// 返回给前端的编译结果（JSON 序列化）
#[derive(serde::Serialize)]
pub struct WebCompileResult {
    pub success: bool,
    pub error_message: Option<String>,
    pub blocks: HashMap<String, String>,
}

/// 前端自动补全需要的元数据
#[derive(serde::Serialize)]
pub struct WebAutocompleteMetadata {
    pub macro_names: Vec<String>,
    pub macro_details: HashMap<String, Vec<String>>,
}

/// 将 ROP 源代码编译为十六进制字符串并返回
#[wasm_bindgen]
pub fn compile_for_web(source_code: &str) -> JsValue {
    let mut result = WebCompileResult {
        success: false,
        error_message: None,
        blocks: HashMap::new(),
    };

    let ast_tree = match parser::parse_to_ast(source_code) {
        Ok(ast) => ast,
        Err(e) => {
            result.error_message = Some(format!("解析失败: {:?}", e));
            return serde_wasm_bindgen::to_value(&result).unwrap();
        }
    };

    let mut compiler = Compiler::new();
    match compiler.compile(&ast_tree) {
        Ok(_) => {
            result.success = true;
            for (block_name, bytes) in compiler.block_outputs {
                let hex_string: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                result.blocks.insert(block_name, hex_string);
            }
        }
        Err(e) => {
            result.error_message = Some(format!("编译失败: {:?}", e));
        }
    }

    serde_wasm_bindgen::to_value(&result).unwrap()
}

/// 从源代码提取宏定义信息，供前端自动补全
#[wasm_bindgen]
pub fn get_autocomplete_metadata(source_code: &str) -> JsValue {
    let mut meta = WebAutocompleteMetadata {
        macro_names: Vec::new(),
        macro_details: HashMap::new(),
    };

    if let Ok(ast_tree) = parser::parse_to_ast(source_code) {
        let mut compiler = Compiler::new();
        for item in &ast_tree.items {
            if let crate::ast::TopLevelItem::MacroDef(m) = item {
                compiler.register_macro(m.clone());
            }
        }

        for (name, def) in compiler.macro_registry {
            meta.macro_names.push(name.clone());
            meta.macro_details.insert(name, def.params);
        }
    }

    serde_wasm_bindgen::to_value(&meta).unwrap()
}