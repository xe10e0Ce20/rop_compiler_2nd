use crate::compiler::Compiler;
use crate::parser;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use js_sys::Function;

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
/// fetch_lib_fn: 前端传入的同步回调函数，签名形如 (lib_name: string) => string
#[wasm_bindgen]
pub fn compile_for_web(source_code: &str, fetch_lib_fn: Function) -> JsValue {
    let mut result = WebCompileResult {
        success: false,
        error_message: None,
        blocks: HashMap::new(),
    };

    // 1. 解析当前编辑器中的主文件代码，生成主 AST 树
    let mut ast_tree = match parser::parse_to_ast(source_code) {
        Ok(ast) => ast,
        Err(e) => {
            result.error_message = Some(format!("主代码解析失败: {:?}", e));
            return serde_wasm_bindgen::to_value(&result).unwrap();
        }
    };

    let mut compiler = Compiler::new();
    let mut imported_items = Vec::new();

    // 2. 遍历主 AST 树，按需拉取云端/本地公共库
    for item in &ast_tree.items {
        if let crate::ast::TopLevelItem::Import(lib_name) = item {
            let this = JsValue::NULL;
            let arg = JsValue::from_str(lib_name);
            
            // 同步阻断回调前端获取库文件的纯文本内容
            if let Ok(js_code_val) = fetch_lib_fn.call1(&this, &arg) {
                if let Some(lib_code) = js_code_val.as_string() {
                    if !lib_code.is_empty() {
                        // 将拉取到的库文件解析为独立的子 AST
                        match parser::parse_to_ast(&lib_code) {
                            Ok(lib_ast) => {
                                // 提取公共库里定义的宏、指令或符号，塞进临时缓冲队列
                                for lib_item in lib_ast.items {
                                    imported_items.push(lib_item);
                                }
                            }
                            Err(e) => {
                                result.error_message = Some(format!(
                                    "公共库 [@import({})] 内部发生语法解析错误: {:?}", 
                                    lib_name, e
                                ));
                                return serde_wasm_bindgen::to_value(&result).unwrap();
                            }
                        }
                    } else {
                        // 库存在但内容为空，暂时允许通过
                        println!("WASM Warning: 发现空的公共库 '@import({})'", lib_name);
                    }
                } else {
                    // 前端未找到匹配的库文件实体（返回了 null / undefined）
                    result.error_message = Some(format!(
                        "未能在云端控制中心或本地缓存中检索到公共库资产: '@import({})'", 
                        lib_name
                    ));
                    return serde_wasm_bindgen::to_value(&result).unwrap();
                }
            } else {
                result.error_message = Some(format!("前端库检索闭包 VFS 回调异常: '@import({})'", lib_name));
                return serde_wasm_bindgen::to_value(&result).unwrap();
            }
        }
    }

    // 3. 将加载到的所有公共库 AST 节点，静默拼接到主 AST 树的最前端
    // 这样做既完成了宏定义的注册，又绝对不会由于文本拼接而产生行号错位、偏移错乱问题！
    ast_tree.items.splice(0..0, imported_items);

    // 4. 交给现有的编译器核心开始执行两遍扫描编译管道
    match compiler.compile(&ast_tree) {
        Ok(_) => {
            result.success = true;
            println!("Compiler: 编译成功。共 {} 个 block。", compiler.block_outputs.len());
            for (block_name, bytes) in compiler.block_outputs {
                let hex_string: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                result.blocks.insert(block_name, hex_string);
            }
        }
        Err(e) => {
            result.error_message = Some(format!("{:?}", e));
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