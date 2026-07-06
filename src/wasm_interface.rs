// src/wasm_interface.rs
use crate::compiler::Compiler;
use crate::parser;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use js_sys::Function;

#[derive(serde::Serialize)]
pub struct WebCompileResult {
    pub success: bool,
    pub error_message: Option<String>,
    pub line: Option<usize>,      // 新增：出错目标行 (从 1 开始计算)
    pub column: Option<usize>,    // 新增：出错目标列 (从 1 开始计算)
    pub blocks: HashMap<String, String>,
}

/// 工具函数：根据代码全局字节偏移量计算其在前端编辑器中的 Line 与 Column 坐标
fn calculate_position(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (idx, c) in source.chars().enumerate() {
        if idx >= byte_offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// 辅助清洗提取真实错误数据
fn handle_rope_error(err: miette::Error, source_code: &str, result: &mut WebCompileResult) {
    // 正确下转：先解引用为 trait object，再 downcast
    let err_ref: &(dyn std::error::Error + 'static) = &*err;
    if let Some(rop_err) = err_ref.downcast_ref::<crate::errors::RopError>() {
        match rop_err {
            crate::errors::RopError::SyntaxError { message, span } => {
                let (l, c) = calculate_position(source_code, span.offset());
                result.line = Some(l);
                result.column = Some(c);
                result.error_message = Some(format!(
                    "[行 {}, 列 {}] [Line {}, Column {}] 语法解析错误 / Syntax parsing error: {}",
                    l, c, l, c, message
                ));
            }
            crate::errors::RopError::CompileError { message, span } => {
                let (l, c) = calculate_position(source_code, span.offset());
                result.line = Some(l);
                result.column = Some(c);
                result.error_message = Some(format!(
                    "[行 {}, 列 {}] [Line {}, Column {}] 编译语义错误 / Compilation semantic error: {}",
                    l, c, l, c, message
                ));
            }
        }
    } else {
        result.error_message = Some(format!("未分类异常 / Unclassified error: {:?}", err));
    }
}

#[wasm_bindgen]
pub fn compile_for_web(source_code: &str, fetch_lib_fn: Function) -> JsValue {
    let mut result = WebCompileResult {
        success: false,
        error_message: None,
        line: None,
        column: None,
        blocks: HashMap::new(),
    };

    // 1. 纯净解析主文件 AST，此时它的所有 Span 偏移量与 source_code 完美一一对应
    let ast_tree = match parser::parse_to_ast(source_code) {
        Ok(ast) => ast,
        Err(e) => {
            handle_rope_error(e, source_code, &mut result);
            return serde_wasm_bindgen::to_value(&result).unwrap();
        }
    };

    let mut compiler = Compiler::new();

    // 2. 独立解析公共库，只把宏注册进 compiler，绝不污染主文件的 items
    for item in &ast_tree.items {
        if let crate::ast::TopLevelItem::Import(lib_name) = item {
            let this = JsValue::NULL;
            let arg = JsValue::from_str(lib_name);
            
            if let Ok(js_code_val) = fetch_lib_fn.call1(&this, &arg) {
                if let Some(lib_code) = js_code_val.as_string() {
                    if !lib_code.is_empty() {
                        match parser::parse_to_ast(&lib_code) {
                            Ok(lib_ast) => {
                                // 仅仅注册公共库里的宏，不拼接到主 AST 树前！
                                for lib_item in lib_ast.items {
                                    if let crate::ast::TopLevelItem::MacroDef(m) = lib_item {
                                        compiler.register_macro(m);
                                    }
                                }
                            }
                            Err(e) => {
                                handle_rope_error(e, &lib_code, &mut result);
                                result.error_message = Some(format!("公共库 [{}] 语法错误: {}", lib_name, result.error_message.unwrap_or_default()));
                                return serde_wasm_bindgen::to_value(&result).unwrap();
                            }
                        }
                    }
                } else {
                    result.error_message = Some(format!("找不到公共库资产: '@import({})'", lib_name));
                    return serde_wasm_bindgen::to_value(&result).unwrap();
                }
            }
        }
    }

    // 3. 执行编译（内部不再重复注册宏，直接扫描主树 items 即可）
    match compiler.compile(&ast_tree) {
        Ok(_) => {
            result.success = true;
            for (block_name, bytes) in compiler.block_outputs {
                let hex_string: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                result.blocks.insert(block_name, hex_string);
            }
        }
        Err(e) => {
            // 4. 这里的 e 内部由于没有被公共库污染，绝对能算对行号
            handle_rope_error(e, source_code, &mut result);
        }
    }

    serde_wasm_bindgen::to_value(&result).unwrap()
}

// ----------------- 以下保持原有逻辑不变 -----------------
#[derive(serde::Serialize)]
pub struct WebAutocompleteMetadata {
    pub macro_names: Vec<String>,
    pub macro_details: HashMap<String, Vec<String>>,
}

#[wasm_bindgen]
pub fn get_autocomplete_metadata(source_code: &str) -> JsValue {
    let mut meta = WebAutocompleteMetadata {
        macro_names: Vec::new(),
        macro_details: HashMap::new(),
    };
    if let Ok(ast_tree) = parser::parse_to_ast(source_code) {
        let mut compiler = Compiler::new();
        for item in &ast_tree.items {
            if let crate::ast::TopLevelItem::MacroDef(m) = item { compiler.register_macro(m.clone()); }
        }
        for (name, def) in compiler.macro_registry {
            meta.macro_names.push(name.clone());
            meta.macro_details.insert(name, def.params);
        }
    }
    serde_wasm_bindgen::to_value(&meta).unwrap()
}