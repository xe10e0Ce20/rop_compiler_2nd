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
    pub symbol_table: HashMap<String, u16>,          // 绝对地址
    pub raw_symbol_table: HashMap<String, u16>,      // 相对偏移（定义时的块内偏移）
    pub macro_call_counter: HashMap<String, usize>,
    pub current_filler: char,
    pub span_map: HashMap<String, Vec<(Range<usize>, Range<usize>)>>,
    block_base: HashMap<String, u16>,
    block_offset: HashMap<String, u16>,
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
            span_map: HashMap::new(),
            block_base: HashMap::new(),
            block_offset: HashMap::new(),
        }
    }

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

    fn rename_local_labels_in_node(node: &mut Node, name: &str, call_id: usize) {
        match node {
            Node::Label(n) if n.starts_with('_') => {
                *n = format!("__{}_{}_{}", name, call_id, n);
            }
            Node::Value(spanned_expr) => {
                Self::rename_local_labels_in_expr(&mut spanned_expr.node, name, call_id);
            }
            Node::MacroCall { args, body, .. } => {
                for arg in args.iter_mut() {
                    Self::rename_local_labels_in_expr(&mut arg.node, name, call_id);
                }
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
        self.block_base.clear();
        self.block_offset.clear();
        self.run_pass(ast, true)?;

        self.block_base.clear();
        self.block_offset.clear();
        self.current_offset = 0;
        self.base_address = 0;
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
        let name = block.name.clone();

        if let Some(&base) = self.block_base.get(&name) {
            self.base_address = base;
        } else {
            self.base_address = 0;
        }
        if let Some(&offset) = self.block_offset.get(&name) {
            self.current_offset = offset;
        } else {
            self.current_offset = 0;
        }

        if dry_run {
            let abs_addr = self.base_address + self.current_offset;
            self.symbol_table.insert(name.clone(), abs_addr);
            // block 名称本身也可以作为标签引用，其相对偏移为当前偏移（通常为0）
            self.raw_symbol_table.insert(name.clone(), self.current_offset);
        } else {
            self.block_outputs.entry(name.clone()).or_insert(Vec::new());
        }

        let old_block = self.current_block_name.clone();
        self.current_block_name = Some(name.clone());

        for spanned_node in &block.contents {
            self.process_node(&spanned_node.node, &spanned_node.span, &None, &HashMap::new(), dry_run)?;
        }

        self.block_base.insert(name.clone(), self.base_address);
        self.block_offset.insert(name.clone(), self.current_offset);

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

        // 0x 开头的十六进制数值（任意长度）
        if target_key.starts_with("0x") {
            let hex_str = target_key.trim_start_matches("0x");
            let hex_str: String = hex_str.chars().filter(|c| !c.is_whitespace()).collect();
            if hex_str.is_empty() {
                return Err(RopError::CompileError {
                    message: "空的十六进制数值 / Empty hex value".into(),
                    span: (span.start, span.len()).into(),
                }.into());
            }
            let mut bytes = Vec::new();
            let mut chars = hex_str.chars().peekable();
            while let Some(c1) = chars.next() {
                let c2 = chars.next().unwrap_or('0');
                let byte = u8::from_str_radix(&format!("{}{}", c1, c2), 16).map_err(|e| {
                    RopError::CompileError {
                        message: format!("无效的十六进制数值 / Invalid hex value: {}", e),
                        span: (span.start, span.len()).into(),
                    }
                })?;
                bytes.push(byte);
            }
            return Ok(RopValue::from_bytes(bytes));
        }

        // 严格 2 位十六进制字节（如 "A8"）
        if target_key.len() == 2 && target_key.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u8::from_str_radix(&target_key, 16).map_err(|e| {
                RopError::CompileError {
                    message: format!("无效的字节值 / Invalid byte value: {}", e),
                    span: (span.start, span.len()).into(),
                }
            })?;
            return Ok(RopValue::from_u64(v as u64, 1));
        }

        // 参数变量
        if let Some(v) = arg_env.get(&target_key) {
            return Ok(v.clone());
        }

        // 标签/符号引用
        if is_raw {
            // & 前缀：返回定义时的相对偏移（固化值）
            if let Some(&raw) = self.raw_symbol_table.get(&target_key) {
                return Ok(RopValue::from_u64(raw as u64, 2));
            }
        } else {
            // 无前缀：返回绝对地址
            if let Some(&addr) = self.symbol_table.get(&target_key) {
                return Ok(RopValue::from_u64(addr as u64, 2));
            }
        }

        if dry_run {
            return Ok(RopValue::placeholder(2));
        }

        Err(RopError::CompileError {
            message: format!("未定义的标识符 '{}' / Undefined identifier '{}'", target_key, target_key),
            span: (span.start, span.len()).into(),
        }.into())
    }

    fn evaluate_expr(&self, expr: &Expr, span: &Range<usize>, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        if expr.tokens.is_empty() {
            return Ok(RopValue::placeholder(0));
        }

        let eval_token = |t: &ExprToken| -> Result<RopValue> {
            match t {
                ExprToken::Raw(s) => self.parse_single_token(s, arg_env, dry_run, span),
                ExprToken::Group(inner) => self.evaluate_expr(inner, span, arg_env, dry_run),
                ExprToken::Bracket(inner) => {
                    let val = self.evaluate_expr(inner, span, arg_env, dry_run)?;
                    Ok(val.reverse_bytes())
                }
                ExprToken::Op(_) => Err(RopError::CompileError {
                    message: "语法错误：此处不应出现操作符 / Syntax error: operator should not appear here".to_string(),
                    span: (span.start, span.len()).into(),
                }.into()),
            }
        };

        let mut current = eval_token(&expr.tokens[0])?;
        let mut i = 1;

        while i + 1 < expr.tokens.len() {
            let op = match &expr.tokens[i] {
                ExprToken::Op(o) => o,
                _ => return Err(RopError::CompileError {
                    message: "语法错误：此处应该是一个操作符 / Syntax error: expected an operator".to_string(),
                    span: (span.start, span.len()).into(),
                }.into()),
            };
            let next = eval_token(&expr.tokens[i + 1])?;

            current = match op.as_str() {
                "+" | "-" => current.math_op(&next, op).map_err(|e| RopError::CompileError {
                    message: e,
                    span: (span.start, span.len()).into(),
                })?,
                "|" => current.concat(&next),
                _ => return Err(RopError::CompileError {
                    message: format!("不支持的运算符 / Unsupported operator: {}", op),
                    span: (span.start, span.len()).into(),
                }.into()),
            };
            i += 2;
        }

        if i < expr.tokens.len() {
            return Err(RopError::CompileError {
                message: "表达式存在未配对的操作符 / Expression has unmatched operator".to_string(),
                span: (span.start, span.len()).into(),
            }.into());
        }

        Ok(current)
    }

    fn process_node(&mut self, node: &Node, node_span: &Range<usize>, trailing_body: &Option<Vec<Spanned<Node>>>, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<()> {
        match node {
            Node::Label(name) => {
                if dry_run {
                    let abs_addr = self.base_address + self.current_offset;
                    self.symbol_table.insert(name.clone(), abs_addr);
                    // 存储相对偏移（固化值，不受后续基址变化影响）
                    self.raw_symbol_table.insert(name.clone(), self.current_offset);
                }
                Ok(())
            }
            Node::Instruction(inst) => self.process_instruction(inst, node_span),
            Node::Value(spanned_expr) => {
                let rop_val = self.evaluate_expr(&spanned_expr.node, &spanned_expr.span, arg_env, dry_run)?;
                if !dry_run {
                    if let Some(ref block_name) = self.current_block_name {
                        let start_off = self.block_outputs.get(block_name).map(|v| v.len()).unwrap_or(0);
                        let bytes = &rop_val.val;
                        let len = bytes.len();
                        self.span_map.entry(block_name.clone())
                            .or_default()
                            .push((spanned_expr.span.clone(), start_off..start_off + len));
                        if let Some(output) = self.block_outputs.get_mut(block_name) {
                            output.extend_from_slice(bytes);
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
                let start_off = if !dry_run {
                    self.current_block_name.as_ref()
                        .and_then(|n| self.block_outputs.get(n).map(|v| v.len()))
                        .unwrap_or(0)
                } else {
                    0
                };

                let count = self.macro_call_counter.entry(name.clone()).or_insert(0);
                *count += 1;
                let call_id = *count;

                let mut macro_def = self.macro_registry.get(name).cloned().ok_or_else(|| {
                    RopError::CompileError {
                        message: format!("未定义的宏 / Undefined macro: {}", name),
                        span: (node_span.start, node_span.len()).into(),
                    }
                })?;

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
                        let spanned_expr = &args[i];
                        self.evaluate_expr(&spanned_expr.node, &spanned_expr.span, arg_env, dry_run)?
                    } else {
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

                for wrapper in macro_def.body.iter_mut() {
                    Self::rename_local_labels_in_node(&mut wrapper.node, name, call_id);
                }

                for n in &macro_def.body {
                    self.process_node(&n.node, &n.span, body, &next_env, dry_run)?;
                }

                if !dry_run {
                    if let Some(ref block_name) = self.current_block_name {
                        let end_off = self.block_outputs.get(block_name).map(|v| v.len()).unwrap_or(start_off);
                        if end_off > start_off {
                            self.span_map.entry(block_name.clone())
                                .or_default()
                                .push((node_span.clone(), start_off..end_off));
                        }
                    }
                }
                Ok(())
            }
        }
    }
}