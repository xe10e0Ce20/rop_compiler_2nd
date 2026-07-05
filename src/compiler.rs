use std::collections::HashMap;
use miette::{miette, Result};
use crate::ast::*;

pub struct Compiler {
    pub base_address: u16,
    pub current_offset: u16,
    // 每个 block 独立的输出缓冲区
    pub block_outputs: HashMap<String, Vec<u8>>,
    // 当前正在编译的 block 名称
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

    pub fn register_macro(&mut self, def: MacroDef) {
        self.macro_registry.insert(def.name.clone(), def);
    }

    // 前端自动补全元数据提取
    pub fn get_autocomplete_metadata(&self) -> Vec<(String, Vec<String>)> {
        self.macro_registry
            .iter()
            .map(|(name, def)| (name.clone(), def.params.clone()))
            .collect()
    }

    pub fn compile(&mut self, ast: &RopFile) -> Result<u16> {
        // 1. 注册宏
        for item in &ast.items {
            if let TopLevelItem::MacroDef(m) = item {
                self.register_macro(m.clone());
            }
        }

        // 2. 第一遍：Dry Run 建立符号表
        self.run_pass(ast, true)?;

        // 3. 重置状态，准备第二遍
        self.current_offset = self.base_address;
        self.block_outputs.clear();
        self.macro_call_counter.clear();
        self.current_block_name = None;

        // 4. 第二遍：生成代码
        self.run_pass(ast, false)?;

        Ok(self.base_address)
    }

    fn run_pass(&mut self, ast: &RopFile, dry_run: bool) -> Result<()> {
        for item in &ast.items {
            match item {
                TopLevelItem::Instruction(inst) => {
                    self.process_instruction(inst, dry_run)?;
                },
                TopLevelItem::Block(block) => self.process_block(block, dry_run)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn process_instruction(&mut self, inst: &Instruction, dry_run: bool) -> Result<()> {
        // 指令必须位于 block 内部
        if self.current_block_name.is_none() {
            return Err(miette!("语法错误：指令必须定义在 block 内部 / Syntax error: instruction must be inside a block"));
        }

        match inst {
            Instruction::Offset(new_offset) => {
                if dry_run {
                    self.base_address = *new_offset;
                }
            }
            Instruction::SetFiller(c) => {
                self.current_filler = *c;
            }
        }  
        Ok(())
    }

    // 处理 block，支持隔离输出
    fn process_block(&mut self, block: &Block, dry_run: bool) -> Result<()> {
        if dry_run {
            self.symbol_table.insert(block.name.clone(), self.current_offset);
        } else {
            self.block_outputs.entry(block.name.clone()).or_insert(Vec::new());
        }

        let old_block = self.current_block_name.clone();
        self.current_block_name = Some(block.name.clone());

        for node in &block.contents {
            self.process_node(node, &None, &HashMap::new(), dry_run)?;
        }

        self.current_block_name = old_block;
        Ok(())
    }

    fn resolve_placeholders(&self, input: &str) -> String {
        input.replace('.', &self.current_filler.to_string())
    }

    fn parse_single_token(&self, t: &str, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        let t_owned = self.resolve_placeholders(t);
        let is_raw = t_owned.starts_with('&');
        let target_key = if is_raw { t_owned[1..].to_string() } else { t_owned };

        if target_key.starts_with("0x") {
            let hex_str = target_key.trim_start_matches("0x");
            let v = u64::from_str_radix(hex_str, 16)
                .map_err(|e| miette!("无效Hex: {0} / Invalid hex: {0}", e))?;
            let len = (hex_str.len() + 1) / 2;
            return Ok(RopValue::new(v, len));
        }
        
        if target_key.len() == 2 && target_key.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u64::from_str_radix(&target_key, 16)
                .map_err(|e| miette!("无效Byte: {0} / Invalid byte: {0}", e))?;
            return Ok(RopValue::new(v, 1));
        } 

        if let Some(&v) = arg_env.get(&target_key) {
            Ok(v)
        } else if let Some(&v) = self.symbol_table.get(&target_key) {
            let final_val = if is_raw {
                (v - self.base_address) as u64 
            } else {
                v as u64
            };
            Ok(RopValue::new(final_val, 2))
        } else if dry_run {
            Ok(RopValue::new(0, 2))
        } else {
            Err(miette!("未定义的标识符 '{0}' / Undefined identifier '{0}'", target_key))
        }
    }

    fn rename_expr_labels(&self, expr: &mut Expr, call_id: usize, macro_name: &str) {
        for token in expr.tokens.iter_mut() {
            match token {
                ExprToken::Raw(s) if s.starts_with('_') => {
                    *s = format!("__{}_{}_{}", macro_name, call_id, s);
                }
                ExprToken::Bracket(inner_expr) | ExprToken::Group(inner_expr) => {
                    self.rename_expr_labels(inner_expr, call_id, macro_name);
                }
                _ => {}
            }
        }
    }

    fn expand_macros_recursive(&mut self, nodes: &mut Vec<Node>, call_id: usize, macro_name: &str) {
        for node in nodes.iter_mut() {
            match node {
                Node::Label(name) if name.starts_with('_') => {
                    *name = format!("__{}_{}_{}", macro_name, call_id, name);
                },
                Node::Value(expr) => {
                    self.rename_expr_labels(expr, call_id, macro_name);
                },
                _ => {}
            }
        }
    }

    fn evaluate_expr(&self, expr: &Expr, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        if expr.tokens.is_empty() { return Ok(RopValue::new(0, 0)); }
        
        let eval_token = |t: &ExprToken| -> Result<RopValue> {
            match t {
                ExprToken::Raw(s) => self.parse_single_token(s, arg_env, dry_run),
                ExprToken::Group(inner) => self.evaluate_expr(inner, arg_env, dry_run),
                ExprToken::Bracket(inner) => {
                    let mut val = self.evaluate_expr(inner, arg_env, dry_run)?;
                    val.val = RopValue::reverse_rop_bytes(val.val, val.len);
                    Ok(val)
                }
                ExprToken::Op(_) => Err(miette!("语法错误：此处不应出现操作符 / Syntax error: operator unexpected here")),
            }
        };

        let mut current = eval_token(&expr.tokens[0])?;
        let mut i = 1;

        while i + 1 < expr.tokens.len() {
            let op = match &expr.tokens[i] {
                ExprToken::Op(o) => o,
                _ => return Err(miette!("语法错误：期待操作符 / Syntax error: expected operator")),
            };
            let next = eval_token(&expr.tokens[i + 1])?;
            
            current = match op.as_str() {
                "+" | "-" => current.math_op(&next, op),
                "|" => current.concat(&next),
                _ => return Err(miette!("不支持的运算符: {0} / Unsupported operator: {0}", op)),
            };
            i += 2;
        }

        if i < expr.tokens.len() {
            return Err(miette!("表达式存在未配对的操作符 / Expression has unmatched operator"));
        }
        Ok(current)
    }

    fn process_node(&mut self, node: &Node, trailing_body: &Option<Vec<Node>>, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<()> {
        match node {
            Node::Label(name) => {
                if dry_run { self.symbol_table.insert(name.clone(), self.current_offset); }
                Ok(())
            }
            Node::Instruction(inst) => {
                self.process_instruction(inst, dry_run)
            }
            Node::Value(expr) => {
                let rop_val = self.evaluate_expr(expr, arg_env, dry_run)?;
                
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
                    for n in nodes { self.process_node(n, &None, arg_env, dry_run)?; }
                }
                Ok(())
            }
            Node::MacroCall { name, args, body } => {
                let count = self.macro_call_counter.entry(name.clone()).or_insert(0);
                *count += 1;
                let call_id = *count;

                let mut macro_def = self.macro_registry.get(name)
                    .cloned()
                    .ok_or_else(|| miette!("宏未定义 / Macro undefined"))?;
    
                let mut next_env = arg_env.clone();
                for (i, param) in macro_def.params.iter().enumerate() {
                    if let Some(arg_expr) = args.get(i) {
                        let rop_val = self.evaluate_expr(arg_expr, arg_env, dry_run)?;
                        next_env.insert(param.clone(), rop_val);
                    }
                }

                self.expand_macros_recursive(&mut macro_def.body, call_id, name);

                for n in &macro_def.body {
                    self.process_node(n, body, &next_env, dry_run)?;
                }
                Ok(())
            }
        }
    }
}