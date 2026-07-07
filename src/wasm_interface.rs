// src/wasm_interface.rs
use crate::compiler::Compiler;
use crate::parser;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

#[derive(serde::Serialize)]
pub struct WebCompileResult {
    pub success: bool,
    pub error_message: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub blocks: HashMap<String, String>,
    pub span_map: HashMap<String, Vec<[usize; 4]>>,
}

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

fn handle_rope_error(err: miette::Error, source_code: &str, result: &mut WebCompileResult) {
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

/// 将 UTF-8 字节偏移转换为 UTF-16 码元偏移（Monaco 使用的字符索引）
fn byte_offset_to_utf16_offset(source: &str, byte_offset: usize) -> usize {
    let mut utf16_count = 0;
    let mut current_byte = 0;
    for ch in source.chars() {
        if current_byte >= byte_offset {
            break;
        }
        current_byte += ch.len_utf8();
        if current_byte > byte_offset {
            // 偏移落在一个多字节字符的中间，返回该字符之前的长度
            break;
        }
        utf16_count += ch.len_utf16();
    }
    utf16_count
}

#[wasm_bindgen]
pub fn compile_for_web(source_code: &str, fetch_lib_fn: js_sys::Function) -> JsValue {
    let mut result = WebCompileResult {
        success: false,
        error_message: None,
        line: None,
        column: None,
        blocks: HashMap::new(),
        span_map: HashMap::new(),
    };

    let ast_tree = match parser::parse_to_ast(source_code) {
        Ok(ast) => ast,
        Err(e) => {
            handle_rope_error(e, source_code, &mut result);
            return serde_wasm_bindgen::to_value(&result).unwrap();
        }
    };

    let mut compiler = Compiler::new();

    // 注册导入库中的宏
    for item in &ast_tree.items {
        if let crate::ast::TopLevelItem::Import(lib_name) = item {
            let this = JsValue::NULL;
            let arg = JsValue::from_str(lib_name);
            
            if let Ok(js_code_val) = fetch_lib_fn.call1(&this, &arg) {
                if let Some(lib_code) = js_code_val.as_string() {
                    if !lib_code.is_empty() {
                        match parser::parse_to_ast(&lib_code) {
                            Ok(lib_ast) => {
                                for lib_item in lib_ast.items {
                                    if let crate::ast::TopLevelItem::MacroDef(m) = lib_item {
                                        compiler.register_macro(m);
                                    }
                                }
                            }
                            Err(e) => {
                                handle_rope_error(e, &lib_code, &mut result);
                                result.error_message = Some(format!("公共库 [{}] 语法错误 / Public library [{}] syntax error: {}", lib_name, lib_name, result.error_message.unwrap_or_default()));
                                return serde_wasm_bindgen::to_value(&result).unwrap();
                            }
                        }
                    }
                } else {
                    result.error_message = Some(format!("找不到公共库资产 / Cannot find public library asset: '@import({})'", lib_name));
                    return serde_wasm_bindgen::to_value(&result).unwrap();
                }
            }
        }
    }

    match compiler.compile(&ast_tree) {
        Ok(_) => {
            result.success = true;

            // 填充 blocks
            for (block_name, bytes) in compiler.block_outputs {
                let hex_string: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                result.blocks.insert(block_name, hex_string);
            }

            // 填充 span_map，将字节偏移转换为 UTF-16 字符偏移
            let main_len = source_code.len();
            let span_map: HashMap<String, Vec<[usize; 4]>> = compiler
                .span_map
                .into_iter()
                .map(|(block, vec)| {
                    let filtered: Vec<[usize; 4]> = vec
                        .into_iter()
                        .filter(|(src_span, _)| src_span.start < main_len)
                        .map(|(src_span, out_range)| {
                            let char_start = byte_offset_to_utf16_offset(source_code, src_span.start);
                            let char_end = byte_offset_to_utf16_offset(source_code, src_span.end);
                            [char_start, char_end, out_range.start, out_range.end]
                        })
                        .collect();
                    (block, filtered)
                })
                .collect();
            result.span_map = span_map;
        }
        Err(e) => {
            handle_rope_error(e, source_code, &mut result);
        }
    }

    serde_wasm_bindgen::to_value(&result).unwrap()
}

// ---------- 自动补全元数据 ----------
#[derive(serde::Serialize)]
pub struct WebAutocompleteMetadata {
    pub macro_names: Vec<String>,
    pub macro_details: HashMap<String, Vec<MacroParamInfo>>,
}

#[derive(serde::Serialize)]
pub struct MacroParamInfo {
    pub name: String,
    pub type_spec: Option<String>,
    pub has_default: bool,
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
            if let crate::ast::TopLevelItem::MacroDef(m) = item {
                compiler.register_macro(m.clone());
            }
        }
        for (name, def) in compiler.macro_registry {
            meta.macro_names.push(name.clone());
            let params_info: Vec<MacroParamInfo> = def
                .params
                .iter()
                .map(|p| MacroParamInfo {
                    name: p.name.clone(),
                    type_spec: p.type_spec.as_ref().map(|ts| format!("{}b", ts.byte_len)),
                    has_default: p.default.is_some(),
                })
                .collect();
            meta.macro_details.insert(name, params_info);
        }
    }
    serde_wasm_bindgen::to_value(&meta).unwrap()
}