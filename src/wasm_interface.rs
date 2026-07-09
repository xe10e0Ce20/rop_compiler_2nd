// src/wasm_interface.rs
use crate::ast::TopLevelItem;
use crate::ast::Node;
use crate::compiler::Compiler;
use crate::parser;
use std::collections::HashMap;
use std::collections::HashSet; 
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


fn set_spans_in_node(node: &mut Node, span: std::ops::Range<usize>) {
    match node {
        Node::Value(spanned_expr) => {
            spanned_expr.span = span;
        }
        Node::MacroCall { args, body, .. } => {
            for arg in args.iter_mut() {
                arg.span = span.clone();
            }
            if let Some(b) = body {
                for child in b.iter_mut() {
                    child.span = span.clone();
                    set_spans_in_node(&mut child.node, span.clone());
                }
            }
        }
        Node::Instruction(_) => {}   // Node::Instruction 本身不带 span
        Node::Label(_) => {}
        Node::Yield => {}
    }
}

fn set_spans_in_item(item: &mut TopLevelItem, span: std::ops::Range<usize>) {
    match item {
        TopLevelItem::Block(block) => {
            for spanned in block.contents.iter_mut() {
                spanned.span = span.clone();
                set_spans_in_node(&mut spanned.node, span.clone());
            }
        }
        TopLevelItem::MacroDef(mdef) => {
            for spanned in mdef.body.iter_mut() {
                spanned.span = span.clone();
                set_spans_in_node(&mut spanned.node, span.clone());
            }
        }
        TopLevelItem::Instruction(_, s) => *s = span,
        TopLevelItem::Include(_, s) => *s = span,
    }
}


fn calculate_position(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    let mut current_byte = 0;
    for ch in source.chars() {
        if current_byte >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        current_byte += ch.len_utf8();
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

/// 递归展开 AST 中的 @include 指令，原地替换为被包含文件的顶层项
fn expand_includes(
    items: &mut Vec<TopLevelItem>,
    fetch_lib: &js_sys::Function,
    stack: &mut HashSet<String>,
) -> Result<(), String> {
    let mut new_items = Vec::new();
    for item in items.drain(..) {
        match item {
            TopLevelItem::Include(lib_name, include_span) => {
                if stack.contains(&lib_name) {
                    return Err(format!("循环包含: {}", lib_name));
                }
                stack.insert(lib_name.clone());

                let this = JsValue::NULL;
                let arg = JsValue::from_str(&lib_name);
                let js_code_val = fetch_lib
                    .call1(&this, &arg)
                    .map_err(|_| format!("获取库失败: {}", lib_name))?;
                let lib_code = js_code_val
                    .as_string()
                    .ok_or_else(|| format!("库非字符串: {}", lib_name))?;

                if lib_code.is_empty() {
                    stack.remove(&lib_name);
                    continue;
                }

                let lib_ast = parser::parse_to_ast(&lib_code)
                    .map_err(|e| format!("库 [{}] 语法错误: {}", lib_name, e))?;

                let mut lib_items = lib_ast.items;
                // 递归展开库自身的 include
                expand_includes(&mut lib_items, fetch_lib, stack)?;

                for item in lib_items.iter_mut() {
                    set_spans_in_item(item, include_span.clone());
                }

                new_items.extend(lib_items);
                stack.remove(&lib_name);
            }
            other => new_items.push(other),
        }
    }
    *items = new_items;
    Ok(())
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

    // 1. 解析主文件
    let mut ast_tree = match parser::parse_to_ast(source_code) {
        Ok(ast) => ast,
        Err(e) => {
            handle_rope_error(e, source_code, &mut result);
            return serde_wasm_bindgen::to_value(&result).unwrap();
        }
    };

    // 2. 原地展开所有 @include
    let mut include_stack = HashSet::new();
    if let Err(err_msg) = expand_includes(&mut ast_tree.items, &fetch_lib_fn, &mut include_stack) {
        result.error_message = Some(err_msg);
        return serde_wasm_bindgen::to_value(&result).unwrap();
    }

    // 3. 编译（compiler 内部会注册所有宏定义，无需提前处理）
    let mut compiler = Compiler::new();
    match compiler.compile(&ast_tree) {
        Ok(_) => {
            result.success = true;
            // 填充 blocks
            for (block_name, bytes) in compiler.block_outputs {
                let hex_string: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                result.blocks.insert(block_name, hex_string);
            }
            // 填充 span_map
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

// ---------- 自动补全元数据 (保持不变) ----------
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
    // 注意：这里没有展开 include，所以补全只基于当前文件的宏定义。
    // 如需包含库中的宏，可类似 compile_for_web 那样先展开再扫描。
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