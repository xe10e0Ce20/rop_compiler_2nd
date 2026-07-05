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

    fn parse_single_token(&self, t: &str, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        // 0. 处理后缀 .se
        let has_se = t.ends_with(".se");
        let base_t = if has_se { t.trim_end_matches(".se") } else { t };
        
        let is_raw = base_t.starts_with('&');
        let target_key = if is_raw { &base_t[1..] } else { base_t };

        // 1. 处理 Hex 常量 (0x 开头) 或 Byte (如 "00", "A8")
        let mut val = if target_key.starts_with("0x") {
            let hex_str = target_key.trim_start_matches("0x");
            let v = u64::from_str_radix(hex_str, 16).map_err(|e| miette!("无效Hex: {}", e))?;
            let len = (hex_str.len() + 1) / 2;
            RopValue::new(v, len)
        } else if target_key.len() == 2 && target_key.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u64::from_str_radix(target_key, 16).map_err(|e| miette!("无效Byte: {}", e))?;
            RopValue::new(v, 1)
        } else {
            // 2. 查找逻辑
            if let Some(&v) = arg_env.get(target_key) {
                v
            } else if let Some(&v) = self.symbol_table.get(target_key) {
                RopValue::new(v as u64, 2)
            } else if dry_run {
                RopValue::new(0, 2)
            } else {
                return Err(miette!("未定义的标识符 '{}'", base_t));
            }
        };

        // 3. 应用 .se 逻辑：如果在 Token 级别检测到了 .se，直接翻转数值
        // 注意：这里的翻转是针对其长度 len 的反转
        if has_se {
            let bytes = val.val.to_be_bytes(); // 取8字节大端原始值
            let mut target_bytes = bytes[8 - val.len..].to_vec();
            target_bytes.reverse();
            
            // 重新组装回 u64 (作为大端数值存储，以便 | 拼接)
            let mut new_val = 0u64;
            for (i, &b) in target_bytes.iter().enumerate() {
                new_val |= (b as u64) << ((val.len - 1 - i) * 8);
            }
            val.val = new_val;
        }

        Ok(val)
    }

    fn expand_macros_recursive(&mut self, nodes: &mut Vec<Node>, call_id: usize, macro_name: &str) {
        for node in nodes.iter_mut() {
            match node {
                Node::Label(name) if name.starts_with('_') => {
                    // 直接修改标签名，完成名字替换
                    println!("DEBUG: 正在重命名宏内部标签 '{}' -> '__{}_{}_{}'", name, macro_name, call_id, name);
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

    fn evaluate_expr(&self, expr: &Expr, arg_env: &HashMap<String, RopValue>, dry_run: bool) -> Result<RopValue> {
        if expr.raw_tokens.is_empty() { return Ok(RopValue::new(0, 0)); }
        
        let mut current = self.parse_single_token(&expr.raw_tokens[0], arg_env, dry_run)?;
        
        let mut i = 1;
        while i < expr.raw_tokens.len() {
            let op = &expr.raw_tokens[i];
            let next = self.parse_single_token(&expr.raw_tokens[i+1], arg_env, dry_run)?;
            
            current = match op.as_str() {
                "+" | "-" => current.math_op(&next, op),
                "|" => current.concat(&next),
                _ => return Err(miette!("不支持的运算符: {}", op)),
            };
            i += 2;
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

                // 确保这里的 next_env 是 HashMap<String, RopValue>
                let mut next_env: HashMap<String, RopValue> = arg_env.clone(); 
                
                for (i, param) in macro_def.params.iter().enumerate() {
                    if let Some(arg) = args.get(i) {
                        // 这里 evaluate_expr 返回的就是 RopValue
                        let rop_val = self.evaluate_expr(arg, arg_env, dry_run)?;
                        
                        // 【关键修改】：不要用 .val as u16，直接存入整个 rop_val！
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