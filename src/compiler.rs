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
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            base_address: 0,
            current_offset: 0,
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
                // 如果是第一遍且还没设置基址，记录它
                if dry_run && self.base_address == 0 {
                    self.base_address = *new_offset;
                }
                
                if !dry_run {
                    // 原有的偏移检查逻辑
                    if *new_offset < self.current_offset {
                        return Err(miette!("offset 试图回退地址"));
                    }
                    let gap = (*new_offset - self.current_offset) as usize;
                    self.output.resize(self.output.len() + gap, 0x00);
                }
                self.current_offset = *new_offset;
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

    fn parse_single_token(&self, t: &str, arg_env: &HashMap<String, u16>, dry_run: bool) -> Result<(u16, bool)> {
        if t.starts_with("0x") {
            let val = u16::from_str_radix(t.trim_start_matches("0x"), 16).map_err(|e| miette!("无效十六进制数 '{}': {}", t, e))?;
            Ok((val, false))
        } else if t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit()) {
            let val = u16::from_str_radix(t, 16).unwrap();
            Ok((val, true))
        } else if let Some(&val) = arg_env.get(t) {
            Ok((val, false))
        } else if let Some(&val) = self.symbol_table.get(t) {
            Ok((val, false))
        } else {
            // 【关键修复】：
            if dry_run {
                // 如果是 Dry Run，没找到标签就返回 0，别报错！
                Ok((0, false)) 
            } else {
                // 只有在 Final Run 还没找到，才真的是未定义
                Err(miette!("未定义的标识符 '{}'", t))
            }
        }
    }

    fn expand_macros_recursive(&mut self, nodes: &mut Vec<Node>, call_id: usize, macro_name: &str) {
        for node in nodes.iter_mut() {
            println!("DEBUG: 正在检查节点: {:?}", node);
            match node {
                Node::Label(name) if name.starts_with('_') => {
                    // 直接修改标签名，完成名字替换
                    println!("正在重命名宏内部标签 '{}' -> '__{}_{}_{}'", name, macro_name, call_id, name);
                    println!("DEBUG: Mapping local label '{}' to '{}'", name, format!("__{}_{}{}", macro_name, call_id, name));
                    *name = format!("__{}_{}_{}", macro_name, call_id, name);
                },
                Node::Value(expr) => {
                    // 同样替换表达式中的引用
                    for token in expr.raw_tokens.iter_mut() {
                        if token.starts_with('_') {
                            *token = format!("__{}_{}_{}", macro_name, call_id, token);
                        }
                    }
                },
                _ => {}
            }
        }
    }

    fn evaluate_expr(&self, expr: &Expr, arg_env: &HashMap<String, u16>, dry_run: bool) -> Result<(u16, bool)> {
        if expr.raw_tokens.is_empty() { return Ok((0, false)); }
        
        // 现在这里可以安全地使用 dry_run 了
        if expr.raw_tokens.len() == 1 {
            return self.parse_single_token(&expr.raw_tokens[0], arg_env, dry_run);
        }

        let (mut total, _) = self.parse_single_token(&expr.raw_tokens[0], arg_env, dry_run)?;
        let mut i = 1;
        while i < expr.raw_tokens.len() {
            let op = &expr.raw_tokens[i];
            let (next_val, _) = self.parse_single_token(&expr.raw_tokens[i+1], arg_env, dry_run)?;
            match op.as_str() {
                "+" => total = total.wrapping_add(next_val),
                "-" => total = total.wrapping_sub(next_val),
                _ => return Err(miette!("不支持的运算符")),
            }
            i += 2;
        }
        Ok((total, false))
    }

    // 还原后的节点处理 (核心：dry_run 控制是否写入)
    fn process_node(&mut self, node: &Node, trailing_body: &Option<Vec<Node>>, arg_env: &HashMap<String, u16>, dry_run: bool) -> Result<()> {
        match node {
            Node::Label(name) => {
                if dry_run { self.symbol_table.insert(name.clone(), self.current_offset); }
                Ok(()) // 显式返回
            }
            Node::Instruction(inst) => {
                self.process_instruction(inst, dry_run) // 这里直接返回 Result，不需要 Ok(())
            }
            Node::Value(expr) => {
                let (val, is_single) = self.evaluate_expr(expr, arg_env, dry_run)?;
                if !dry_run {
                    if is_single && expr.endian.is_none() {
                        self.output.push(val as u8);
                    } else {
                        let is_be = expr.endian.as_deref() == Some("be");
                        let bytes = if is_be { val.to_be_bytes() } else { val.to_le_bytes() };
                        self.output.extend_from_slice(&bytes);
                    }
                }
                self.current_offset += if is_single && expr.endian.is_none() { 1 } else { 2 };
                Ok(()) // 显式返回
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

                let mut next_env = arg_env.clone();
                for (i, param) in macro_def.params.iter().enumerate() {
                    if let Some(arg) = args.get(i) {
                        let (val, _) = self.evaluate_expr(arg, arg_env, dry_run)?;
                        next_env.insert(param.clone(), val);
                    }
                }

                self.expand_macros_recursive(&mut macro_def.body, call_id, name);

                for n in &macro_def.body {
                    self.process_node(n, body, &next_env, dry_run)?;
                }
                Ok(()) // 显式返回
            }
        }
    }
}