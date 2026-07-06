// src/compiler.rs
use std::collections::HashMap;
use std::ops::Range;
use miette::Result;
use crate::ast::*;
use crate::errors::RopError;

pub struct Compiler {
    pub base_address: u16,
    pub current_offset: u16,
    pub block_outputs: HashMap<String, Vec<u8>>,
    pub current_block_name: Option<String>,
    pub macro_registry: HashMap<String, MacroDef>,
    pub symbol_table: HashMap<String, u16>, 
    pub raw_symbol_table: HashMap<String, u16>,
    pub macro_call_counter: HashMap<String, usize>,
    pub current_filler: char,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            base_address: 0,
            current_offset: 0,
            current_filler: '0',
            block_outputs: HashMap::new(),
            current_block_name: None,
            macro_registry: HashMap::new(),
            symbol_table: HashMap::new(),
            raw_symbol_table: HashMap::new(),
            macro_call_counter: HashMap::new(),
        }
    }

    /// 递归重命名表达式 (Expr) 中的局部标签
    /// Recursively rename local labels in expression (Expr)
    fn rename_local_labels_in_expr(expr: &mut Expr, name: &str, call_id: usize) {
        for token in expr.tokens.iter_mut() {
            match token {
                ExprToken::Raw(s) if s.starts_with('_') => {
                    *s = format!("__{}_{}_{}", name, call_id, s);
                }
                ExprToken::Bracket(inner) | ExprToken::Group(inner) => {
                    Self::rename_local_labels_in_expr(inner, name, call_id);
                }
                _ => {}
            }
        }
    }

    /// 递归重命名节点 (Node) 及其所有子作用域中的局部标签
    /// Recursively rename local labels in node and all child scopes
    fn rename_local_labels_in_node(node: &mut Node, name: &str, call_id: usize) {
        match node {
            Node::Label(n) if n.starts_with('_') => {
                *n = format!("__{}_{}_{}", name, call_id, n);
            }
            Node::Value(spanned_expr) => {
                Self::rename_local_labels_in_expr(&mut spanned_expr.node, name, call_id);
            }
            Node::MacroCall { args, body, .. } => {
                // 如果宏的参数里传了局部变量，也要重命名
                // If local variables are passed in macro arguments, rename them too
                for arg in args.iter_mut() {
                    Self::rename_local_labels_in_expr(&mut arg.node, name, call_id);
                }
                // 如果宏自带了 trailing body (大括号里的内容)，继续递归
                // If the macro has a trailing body (content in curly braces), continue recursion
                if let Some(b) = body.as_mut() {
                    for child in b.iter_mut() {
                        Self::rename_local_labels_in_node(&mut child.node, name, call_id);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn register_macro(&mut self, def: MacroDef) {
        self.macro_registry.insert(def.name.clone(), def);
    }

    pub fn compile(&mut self, ast: &RopFile) -> Result<u16> {
        for item in &ast.items {
            if let TopLevelItem::MacroDef(m) = item {
                self.register_macro(m.clone());
            }
        }
        self.run_pass(ast, true)?;
        self.current_offset = self.base_address;
        self.block_outputs.clear();
        self.macro_call_counter.clear();
        self.current_block_name = None;
        self.run_pass(ast, false)?;
        Ok(self.base_address)
    }

    fn run_pass(&mut self, ast: &RopFile, dry_run: bool) -> Result<()> {
        for item in &ast.items {
            match item {
                TopLevelItem::Instruction(inst, span) => {
                    if self.current_block_name.is_none() {
                        return Err(RopError::CompileError {
                            message: "语法错误：指令必须定义在 block 内部 / Syntax error: directive must be defined inside a block".to_string(),
                            span: (span.start, span.len()).into(),
                        }.into());
                    }
                    self.process_instruction(inst, span)?;
                },
                TopLevelItem::Block(block) => self.process_block(block, dry_run)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn process_instruction(&mut self, inst: &Instruction, _span: &Range<usize>) -> Result<()> {
        match inst {
            Instruction::Offset(new_offset) => { self.base_address = *new_offset; }
            Instruction::SetFiller(c) => { self.current_filler = *c; }
        }  
        Ok(())
    }

    fn process_block(&mut self, block: &Block, dry_run: bool) -> Result<()> {
        if dry_run {
            self.symbol_table.insert(block.name.clone(), self.current_offset);
        } else {
            self.block_outputs.entry(block.name.clone()).or_insert(Vec::new());
        }
        let old_block = self.current_block_name.clone();
        self.current_block_name = Some(block.name.clone());

        for spanned_node in &block.contents {
            self.process_node(&spanned_node.node, &spanned_node.span, &None, &HashMap::new(), dry_run)?;
        }
        self.current_block_name = old_block;
        Ok(())
    }

    fn resolve_placeholders(&self, input: &str) -> String {
        input.replace('.', &self.current_filler.to_string())
    }

    fn parse_single_token(&self, t: &str, arg_env: &HashMap<String, RopValue>, dry_run: bool, span: &Range<usize>) -> Result<RopValue> {
        let t_owned = self.resolve_placeholders(t);
        let is_raw = t_owned.starts_with('&');
        let target_key = if is_raw { t_owned[1..].to_string() } else { t_owned };

        if target_key.starts_with("0x") {
            let hex_str = target_key.trim_start_matches("0x");
            let v = u64::from_str_radix(hex_str, 16).map_err(|e| {
                RopError::CompileError { 
                    message: format!("无效的十六进制数值 / Invalid hex value: {}", e), 
                    span: (span.start, span.len()).into() 
                }
            })?;
            return Ok(RopValue::new(v, (hex_str.len() + 1) / 2));
        }
        
        if target_key.len() == 2 && target_key.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u64::from_str_radix(&target_key, 16).map_err(|e| {
                RopError::CompileError { 
                    message: format!("无效的字节值 / Invalid byte value: {}", e), 
                    span: (span.start, span.len()).into() 
                }
            })?;
            return Ok(RopValue::new(v, 1));
        } 

        if let Some(&v) = arg_env.get(&target_key) {
            Ok(v)
        } else if let Some(&v) = self.symbol_table.get(&target_key) {
            let final_val = if is_raw { (v - self.base_address) as u64 } else { v as u64 };
            Ok(RopValue::new(final_val, 2))
        } else if dry_run {
            Ok(RopValue::new(0, 2))
        } else {
            Err(RopError::CompileError {
                message: format!("未定义的标识符 '{}' / Undefined identifier '{}'", target_key, target_key),
                span: (span.start, span.len()).into(),
            }.into())
        }
    }

    fn evaluate_expr(&self, expr: &Expr, span: &Range<usize>, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        if expr.tokens.is_empty() { return Ok(RopValue::new(0, 0)); }
        
        let eval_token = |t: &ExprToken| -> Result<RopValue> {
            match t {
                ExprToken::Raw(s) => self.parse_single_token(s, arg_env, dry_run, span),
                ExprToken::Group(inner) => self.evaluate_expr(inner, span, arg_env, dry_run),
                ExprToken::Bracket(inner) => {
                    let mut val = self.evaluate_expr(inner, span, arg_env, dry_run)?;
                    val.val = RopValue::reverse_rop_bytes(val.val, val.len);
                    Ok(val)
                }
                ExprToken::Op(_) => Err(RopError::CompileError{ 
                    message: "语法错误：此处不应出现操作符 / Syntax error: operator should not appear here".to_string(), 
                    span: (span.start, span.len()).into() 
                }.into()),
            }
        };

        let mut current = eval_token(&expr.tokens[0])?;
        let mut i = 1;

        while i + 1 < expr.tokens.len() {
            let op = match &expr.tokens[i] {
                ExprToken::Op(o) => o,
                _ => return Err(RopError::CompileError{ 
                    message: "语法错误：此处应该是一个操作符 / Syntax error: expected an operator".to_string(), 
                    span: (span.start, span.len()).into() 
                }.into()),
            };
            let next = eval_token(&expr.tokens[i + 1])?;
            
            current = match op.as_str() {
                "+" | "-" => current.math_op(&next, op),
                "|" => current.concat(&next),
                _ => return Err(RopError::CompileError{ 
                    message: format!("不支持的运算符 / Unsupported operator: {}", op), 
                    span: (span.start, span.len()).into() 
                }.into()),
            };
            i += 2;
        }
        if i < expr.tokens.len() {
            return Err(RopError::CompileError{ 
                message: "表达式存在未配对的操作符 / Expression has unmatched operator".to_string(), 
                span: (span.start, span.len()).into() 
            }.into());
        }
        Ok(current)
    }

    fn process_node(&mut self, node: &Node, node_span: &Range<usize>, trailing_body: &Option<Vec<Spanned<Node>>>, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<()> {
        match node {
            Node::Label(name) => {
                if dry_run { self.symbol_table.insert(name.clone(), self.current_offset); }
                Ok(())
            }
            Node::Instruction(inst) => self.process_instruction(inst, node_span),
            Node::Value(spanned_expr) => {
                let rop_val = self.evaluate_expr(&spanned_expr.node, &spanned_expr.span, arg_env, dry_run)?;
                if !dry_run {
                    let bytes = rop_val.val.to_be_bytes()[(8 - rop_val.len)..].to_vec();
                    if let Some(ref block_name) = self.current_block_name {
                        if let Some(output) = self.block_outputs.get_mut(block_name) {
                            output.extend_from_slice(&bytes);
                        }
                    }
                }
                self.current_offset += rop_val.len as u16;
                Ok(())
            }
            Node::Yield => {
                if let Some(nodes) = trailing_body {
                    for n in nodes { self.process_node(&n.node, &n.span, &None, arg_env, dry_run)?; }
                }
                Ok(())
            }
            Node::MacroCall { name, args, body } => {
                let count = self.macro_call_counter.entry(name.clone()).or_insert(0);
                *count += 1;
                let call_id = *count;

                let mut macro_def = self.macro_registry.get(name).cloned().ok_or_else(|| {
                    RopError::CompileError { 
                        message: format!("未定义的宏 / Undefined macro: {}", name), 
                        span: (node_span.start, node_span.len()).into() 
                    }
                })?;
    
                // ----- 参数匹配与默认值处理 -----
                let num_params = macro_def.params.len();
                if args.len() > num_params {
                    return Err(RopError::CompileError {
                        message: format!("宏 '{}' 需要 {} 个参数，但提供了 {} 个 / Macro '{}' expects {} arguments, but {} provided",
                                         name, num_params, args.len(), name, num_params, args.len()),
                        span: (node_span.start, node_span.len()).into(),
                    }.into());
                }

                let mut next_env = arg_env.clone();

                for (i, param_def) in macro_def.params.iter().enumerate() {
                    let val = if i < args.len() {
                        // 显式传入的参数
                        let spanned_expr = &args[i];
                        self.evaluate_expr(&spanned_expr.node, &spanned_expr.span, arg_env, dry_run)?
                    } else {
                        // 使用默认值
                        if let Some(ref default_expr) = param_def.default {
                            self.evaluate_expr(default_expr, node_span, &next_env, dry_run)?
                        } else {
                            return Err(RopError::CompileError {
                                message: format!("宏 '{}' 的参数 '{}' 缺少值且没有默认值 / Missing value for parameter '{}' of macro '{}' without default",
                                                 name, param_def.name, param_def.name, name),
                                span: (node_span.start, node_span.len()).into(),
                            }.into());
                        }
                    };

                    // 类型/长度检查
                    if let Some(ref ts) = param_def.type_spec {
                        if val.len != ts.byte_len {
                            return Err(RopError::CompileError {
                                message: format!("宏 '{}' 的参数 '{}' 期望 {} 字节，但得到 {} 字节 / Parameter '{}' of macro '{}' expects {} bytes, but got {} bytes",
                                                 name, param_def.name, ts.byte_len, val.len, param_def.name, name, ts.byte_len, val.len),
                                span: (node_span.start, node_span.len()).into(),
                            }.into());
                        }
                    }

                    next_env.insert(param_def.name.clone(), val);
                }

                // 局部标签重命名
                for wrapper in macro_def.body.iter_mut() {
                    Self::rename_local_labels_in_node(&mut wrapper.node, name, call_id);
                }

                for n in &macro_def.body {
                    self.process_node(&n.node, &n.span, body, &next_env, dry_run)?;
                }
                Ok(())
            }
        }
    }
}