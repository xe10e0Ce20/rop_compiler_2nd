use std::collections::HashMap;
use miette::{miette, Result};
use crate::ast::*;

pub struct Compiler {
    pub base_address: u16,
    pub current_offset: u16,
    pub output: Vec<u8>,
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
            output: Vec::new(),
            macro_registry: HashMap::new(),
            symbol_table: HashMap::new(),
            raw_symbol_table: HashMap::new(),
            macro_call_counter: HashMap::new(),
        }
    }

    pub fn register_macro(&mut self, def: MacroDef) {

        self.macro_registry.insert(def.name.clone(), def);

    }

    pub fn compile(&mut self, ast: &RopFile) -> Result<u16> {
        // 1. 宏注册 (保留)
        for item in &ast.items {
            if let TopLevelItem::MacroDef(m) = item {
                self.register_macro(m.clone());
            }
        }

        // 2. 第一遍：Dry Run (只填符号表，不写 output)
        self.run_pass(ast, true)?;

        // 3. 重置状态用于第二遍
        self.current_offset = self.base_address;
        self.output.clear();
        self.macro_call_counter.clear();
        // 4. 第二遍：Final Run (执行你原有的逻辑，输出字节)
        self.run_pass(ast, false)?;

        Ok(self.base_address)
    }

    fn run_pass(&mut self, ast: &RopFile, dry_run: bool) -> Result<()> {
        for item in &ast.items {
            match item {
                TopLevelItem::Instruction(inst) => self.process_instruction(inst, dry_run)?,
                TopLevelItem::Block(block) => self.process_block(block, dry_run)?,
                _ => {}
            }
        }
        Ok(())
    }

    // 还原后的指令处理
    fn process_instruction(&mut self, inst: &Instruction, dry_run: bool) -> Result<()> {
        match inst {
            Instruction::Offset(new_offset) => {
                if dry_run && self.base_address == 0 {
                    self.base_address = *new_offset;
                }
                
                if !dry_run {
                    if *new_offset < self.current_offset {
                        return Err(miette!("offset 试图回退地址"));
                    }
                    let gap = (*new_offset - self.current_offset) as usize;
                    self.output.resize(self.output.len() + gap, 0x00);
                }
                self.current_offset = *new_offset;
            }
            Instruction::SetFiller(c) => {
                // 在 dry_run 和实际编译阶段都更新填充值
                self.current_filler = *c;
            }
        }
        Ok(())
    }

    // 还原后的块处理
    fn process_block(&mut self, block: &Block, dry_run: bool) -> Result<()> {
        if dry_run {
            self.symbol_table.insert(block.name.clone(), self.current_offset);
        }
        for node in &block.contents {
            self.process_node(node, &None, &HashMap::new(), dry_run)?;
        }
        Ok(())
    }

    fn resolve_placeholders(&self, input: &str) -> String {
        input.replace('.', &self.current_filler.to_string())
    }

    fn parse_single_token(&self, t: &str, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        let t_owned = self.resolve_placeholders(t);
        
        let is_raw = t_owned.starts_with('&');
        let target_key = if is_raw { 
            t_owned[1..].to_string() 
        } else { 
            t_owned 
        };

        if target_key.starts_with("0x") {
            let hex_str = target_key.trim_start_matches("0x");
            let v = u64::from_str_radix(hex_str, 16).map_err(|e| miette!("无效Hex: {}", e))?;
            let len = (hex_str.len() + 1) / 2;
            return Ok(RopValue::new(v, len));
        }
        
        if target_key.len() == 2 && target_key.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u64::from_str_radix(&target_key, 16).map_err(|e| miette!("无效Byte: {}", e))?;
            return Ok(RopValue::new(v, 1));
        } 

        // 【修复点】：在这里对 target_key 使用 &，即传入 &target_key
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
            Err(miette!("未定义的标识符 '{}'", target_key))
        }
    }

    // 辅助递归替换函数，替换 ExprToken 中的局部标签
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

    // 修改现有的 expand_macros_recursive，调用上面那个辅助函数
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

    // 最核心的表达式求值引擎
    fn evaluate_expr(&self, expr: &Expr, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        if expr.tokens.is_empty() { return Ok(RopValue::new(0, 0)); }
        
        // 闭包：处理单个 Token
        let eval_token = |t: &ExprToken| -> Result<RopValue> {
            match t {
                ExprToken::Raw(s) => self.parse_single_token(s, arg_env, dry_run),
                ExprToken::Group(inner) => self.evaluate_expr(inner, arg_env, dry_run),
                ExprToken::Bracket(inner) => {
                    // 遇到括号，先递归计算里面的值，再执行翻转
                    let mut val = self.evaluate_expr(inner, arg_env, dry_run)?;
                    val.val = RopValue::reverse_rop_bytes(val.val, val.len);
                    Ok(val)
                }
                ExprToken::Op(_) => Err(miette!("语法错误：此处不应出现操作符")),
            }
        };

        let mut current = eval_token(&expr.tokens[0])?;
        let mut i = 1;

        while i + 1 < expr.tokens.len() {
            let op = match &expr.tokens[i] {
                ExprToken::Op(o) => o,
                _ => return Err(miette!("语法错误：期待操作符")),
            };
            let next = eval_token(&expr.tokens[i + 1])?;
            
            current = match op.as_str() {
                "+" | "-" => current.math_op(&next, op),
                "|" => current.concat(&next),
                _ => return Err(miette!("不支持的运算符: {}", op)),
            };
            i += 2;
        }

        if i < expr.tokens.len() {
            return Err(miette!("表达式存在未配对的操作符"));
        }

        Ok(current)
    }
    // 还原后的节点处理 (核心：dry_run 控制是否写入)
    fn process_node(&mut self, node: &Node, trailing_body: &Option<Vec<Node>>, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<()> {
        match node {
            Node::Label(name) => {
                if dry_run { self.symbol_table.insert(name.clone(), self.current_offset); }
                Ok(()) // 显式返回
            }
            Node::Instruction(inst) => {
                self.process_instruction(inst, dry_run) // 这里直接返回 Result，不需要 Ok(())
            }
            Node::Value(expr) => {
                // 1. 获取已经处理好字节序的值
                let rop_val = self.evaluate_expr(expr, arg_env, dry_run)?;
                
                // 2. 直接输出字节，不需要再判断 endian 或 se 了
                if !dry_run {
                    // rop_val.val 现在已经是大端序的数值，且 .se 已经在 parse 阶段处理过了
                    // 我们只需根据 rop_val.len 取出对应的字节
                    let bytes = rop_val.val.to_be_bytes()[(8 - rop_val.len)..].to_vec();
                    self.output.extend_from_slice(&bytes);
                }
                
                // 3. 自动追踪偏移量
                self.current_offset += rop_val.len as u16;
                Ok(())
            }
            Node::Yield => {
                if let Some(nodes) = trailing_body {
                    for n in nodes { self.process_node(n, &None, arg_env, dry_run)?; }
                }
                Ok(()) // 显式返回
            }
            Node::MacroCall { name, args, body } => {
                let count = self.macro_call_counter.entry(name.clone()).or_insert(0);
                *count += 1;
                let call_id = *count;

                let mut macro_def = self.macro_registry.get(name).cloned().ok_or_else(|| miette!("宏未定义"))?;
    
                // 这里依然使用 args 传递 Expr，在 expand 阶段直接替换 raw_tokens
                // 这种方式最接近“文本替换”
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